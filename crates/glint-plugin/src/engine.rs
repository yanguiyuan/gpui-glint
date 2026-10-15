use crate::{Plugin, PluginError, Result};
use boa_engine::{Context, JsNativeError, JsResult, JsValue, NativeFunction, Source, js_string};
use boa_gc::{Finalize, Trace};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};
type Callback = Rc<dyn Fn(Value) -> Result<Value>>;
#[derive(Clone, Default)]
pub struct HostRegistry {
    methods: BTreeMap<String, (String, Callback)>,
}
impl HostRegistry {
    /// Each host API requires an explicit capability, independently of JavaScript wrappers.
    pub fn register(
        &mut self,
        name: &str,
        capability: &str,
        callback: impl Fn(Value) -> Result<Value> + 'static,
    ) -> Result<()> {
        if name.is_empty() || capability.is_empty() || self.methods.contains_key(name) {
            return Err(PluginError("Invalid or duplicate host API".into()));
        }
        self.methods
            .insert(name.into(), (capability.into(), Rc::new(callback)));
        Ok(())
    }
    pub fn with_crypto(mut self) -> Self {
        use hmac::{Hmac, Mac};
        use sha2::{Digest, Sha256};
        self.register("crypto.sha256Hex", "crypto", |args| {
            let text = args
                .get(0)
                .and_then(Value::as_str)
                .ok_or_else(|| PluginError("Expected string".into()))?;
            Ok(Value::String(format!(
                "{:x}",
                Sha256::digest(text.as_bytes())
            )))
        })
        .expect("unique builtin");
        self.register("crypto.hmacSha256Hex", "crypto", |args| {
            let key = args
                .get(0)
                .and_then(Value::as_str)
                .ok_or_else(|| PluginError("Expected key".into()))?;
            let data = args
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| PluginError("Expected string".into()))?;
            let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
                .map_err(|_| PluginError("Invalid key".into()))?;
            mac.update(data.as_bytes());
            Ok(Value::String(format!("{:x}", mac.finalize().into_bytes())))
        })
        .expect("unique builtin");
        self
    }
}
#[derive(Trace, Finalize, Clone)]
struct HostState {
    #[unsafe_ignore_trace]
    registry: HostRegistry,
    #[unsafe_ignore_trace]
    grants: BTreeSet<String>,
}
fn host_call(_: &JsValue, args: &[JsValue], cx: &mut Context) -> JsResult<JsValue> {
    let fail = || JsNativeError::typ().with_message("Host API refused").into();
    let name = args
        .first()
        .and_then(JsValue::as_string)
        .ok_or_else(fail)?
        .to_std_string_escaped();
    let encoded = args
        .get(1)
        .and_then(JsValue::as_string)
        .ok_or_else(fail)?
        .to_std_string_escaped();
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(fail());
    }
    let state = cx.get_data::<HostState>().ok_or_else(fail)?;
    let (capability, callback) = state.registry.methods.get(&name).ok_or_else(fail)?;
    if !state.grants.contains(capability) {
        return Err(fail());
    }
    let callback = callback.clone();
    let value = serde_json::from_str(&encoded).map_err(|_| fail())?;
    let result = callback(value).map_err(|_| fail())?;
    let encoded = serde_json::to_string(&result).map_err(|_| fail())?;
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(fail());
    }
    Ok(boa_engine::JsString::from(encoded).into())
}
/// VM guards are defense in depth, not a hard time/total memory sandbox.
#[derive(Clone, Copy)]
pub struct Limits {
    pub loop_iterations: u64,
    pub recursion: usize,
    pub max_payload_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            loop_iterations: 100_000,
            recursion: 128,
            max_payload_bytes: 8 * 1024 * 1024,
        }
    }
}
pub struct PluginRuntime {
    context: Context,
    hooks: BTreeSet<String>,
    limits: Limits,
}
impl PluginRuntime {
    /// Reactivate a pinned approval; changed source or permissions are rejected before evaluation.
    pub fn activate_approved(
        plugin: &Plugin,
        registry: HostRegistry,
        approval: &crate::Approval,
        limits: Limits,
    ) -> Result<Self> {
        if !approval.matches(plugin) {
            return Err(PluginError(
                "Plugin changed; renewed approval required".into(),
            ));
        }
        Self::activate(plugin, registry, approval.capabilities.clone(), limits)
    }
    /// Activation validates grants and loads source. Use a separate instance per plugin invocation.
    pub fn activate(
        plugin: &Plugin,
        registry: HostRegistry,
        grants: BTreeSet<String>,
        limits: Limits,
    ) -> Result<Self> {
        plugin.manifest.validate()?;
        if plugin.source.len() > 256 * 1024 || !plugin.manifest.capabilities.is_subset(&grants) {
            return Err(PluginError("Plugin permissions were not granted".into()));
        }
        // Only requested grants cross into a plugin, even when the host has broader authority.
        let mut context = Context::default();
        context
            .runtime_limits_mut()
            .set_loop_iteration_limit(limits.loop_iterations);
        context
            .runtime_limits_mut()
            .set_recursion_limit(limits.recursion);
        context.insert_data(HostState {
            registry,
            grants: plugin.manifest.capabilities.clone(),
        });
        context
            .register_global_builtin_callable(
                js_string!("__glintHost"),
                2,
                NativeFunction::from_fn_ptr(host_call),
            )
            .map_err(|_| PluginError("Cannot initialize plugin host".into()))?;
        context
            .eval(Source::from_bytes(PRELUDE))
            .map_err(|_| PluginError("Cannot initialize plugin API".into()))?;
        let wrapped = format!(
            "(function(glint) {{ 'use strict';\n{}\n}})(glint);",
            plugin.source
        );
        context
            .eval(Source::from_bytes(&wrapped))
            .map_err(|_| PluginError("Plugin activation failed".into()))?;
        Ok(Self {
            context,
            hooks: plugin.manifest.hooks.clone(),
            limits,
        })
    }
    /// Pure-data hook boundary: host validates output before committing any changes.
    pub fn invoke(&mut self, hook: &str, payload: &Value) -> Result<Value> {
        if !self.hooks.contains(hook) {
            return Err(PluginError("Plugin did not declare this hook".into()));
        }
        let json =
            serde_json::to_string(payload).map_err(|_| PluginError("Invalid hook input".into()))?;
        if json.len() > self.limits.max_payload_bytes {
            return Err(PluginError("Hook input too large".into()));
        }
        let source = format!(
            "JSON.stringify(__glintDispatch({}, JSON.parse({})))",
            serde_json::to_string(hook).unwrap(),
            serde_json::to_string(&json).unwrap()
        );
        let result = self
            .context
            .eval(Source::from_bytes(&source))
            .map_err(|_| PluginError("Plugin hook failed".into()))?;
        let text = result
            .as_string()
            .ok_or_else(|| PluginError("Hook must return JSON data".into()))?
            .to_std_string_escaped();
        if text.len() > self.limits.max_payload_bytes {
            return Err(PluginError("Hook output too large".into()));
        }
        serde_json::from_str(&text).map_err(|_| PluginError("Invalid hook output".into()))
    }
}
const PRELUDE: &str = r#"
const __glintHooks = new Map();
const glint = Object.freeze({
  apiVersion: 1,
  on(name, callback) {
    if (typeof name !== 'string' || typeof callback !== 'function' || __glintHooks.has(name)) throw new Error('Invalid hook');
    __glintHooks.set(name, callback);
  },
  host: Object.freeze({ call(name, ...args) { return JSON.parse(__glintHost(name, JSON.stringify(args))); } })
});
function __glintDispatch(name, data) {
  const callback = __glintHooks.get(name);
  if (!callback) throw new Error('Missing hook');
  const result = callback(data);
  if (result && typeof result.then === 'function') throw new Error('Async hooks are not supported');
  return result;
}
"#;
