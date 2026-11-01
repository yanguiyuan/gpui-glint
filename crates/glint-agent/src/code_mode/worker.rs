//! No Node APIs or OS bindings are installed in this process-local VM.
use boa_engine::{
    Context, JsNativeError, JsResult, JsValue, NativeFunction, Source,
    builtins::promise::PromiseState, js_string, object::builtins::JsPromise,
};
use boa_gc::{Finalize, Trace};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    io::{BufRead, Write},
    rc::Rc,
};
const FRAME: usize = 4 * 1024 * 1024;
#[derive(Default)]
struct Pending {
    calls: Vec<Value>,
    count: u32,
}
#[derive(Clone, Trace, Finalize)]
struct Bridge {
    #[unsafe_ignore_trace]
    pending: Rc<RefCell<Pending>>,
}
fn enqueue(_: &JsValue, args: &[JsValue], cx: &mut Context) -> JsResult<JsValue> {
    let fail = || JsNativeError::typ().with_message("Invalid tool request");
    let name = args
        .first()
        .and_then(JsValue::as_string)
        .ok_or_else(fail)?
        .to_std_string_escaped();
    let text = args
        .get(1)
        .and_then(JsValue::as_string)
        .ok_or_else(fail)?
        .to_std_string_escaped();
    if text.len() > 2 * 1024 * 1024 {
        return Err(fail().into());
    }
    let arguments: Value = serde_json::from_str(&text).map_err(|_| fail())?;
    if !arguments.is_object() {
        return Err(fail().into());
    }
    let bridge = cx.get_data::<Bridge>().ok_or_else(fail)?;
    let mut pending = bridge.pending.borrow_mut();
    if pending.count >= 64 {
        return Err(JsNativeError::typ()
            .with_message("Tool call limit: 64")
            .into());
    }
    pending.count += 1;
    let id = pending.count;
    pending
        .calls
        .push(json!({"type":"call","id":id,"name":name,"arguments":arguments}));
    Ok(id.into())
}
fn send(out: &mut impl Write, value: &Value) -> Result<(), String> {
    let text = value.to_string();
    if text.len() > FRAME {
        return Err("RPC frame exceeds 4MiB".into());
    }
    writeln!(out, "{text}")
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}
fn receive(input: &mut impl BufRead) -> Result<Value, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    input
        .take((FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > FRAME {
        return Err("Invalid RPC frame or closed host".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
const PRELUDE: &str = r#"
(() => {
 const pending = new Map();
 const allowed = new Set(__names);
 const call = (name,args={}) => new Promise((resolve,reject)=>{
   if (!allowed.has(name)) throw new Error('Tool is not available: '+name);
   const id=__enqueue(name,JSON.stringify(args));pending.set(id,{resolve,reject});
 });
 Object.defineProperty(globalThis,'tools',{value:Object.freeze(Object.fromEntries([...allowed].map(name=>[name,(args)=>call(name,args)]))),writable:false,configurable:false});
 return (reply)=>{const p=pending.get(reply.id);if(!p)throw new Error('Unknown RPC id');pending.delete(reply.id);if(reply.success)p.resolve(reply.value);else p.reject(new Error(reply.error||'Tool failed'));};
})()
"#;
pub(super) fn evaluate(input: &mut impl BufRead, out: &mut impl Write) -> Result<(), String> {
    let start = receive(input)?;
    let source = start["code"]
        .as_str()
        .filter(|s| s.len() <= 65536)
        .ok_or("Code limit is 64KiB")?;
    let pending = Rc::new(RefCell::new(Pending::default()));
    let mut cx = Context::builder()
        .module_loader(Rc::new(boa_engine::module::IdleModuleLoader))
        .build()
        .map_err(|e| e.to_string())?;
    cx.runtime_limits_mut().set_loop_iteration_limit(100_000);
    cx.runtime_limits_mut().set_recursion_limit(128);
    cx.runtime_limits_mut().set_stack_size_limit(4096);
    cx.insert_data(Bridge {
        pending: pending.clone(),
    });
    cx.register_global_builtin_callable(
        js_string!("__enqueue"),
        2,
        NativeFunction::from_fn_ptr(enqueue),
    )
    .map_err(|e| e.to_string())?;
    cx.eval(Source::from_bytes(&format!(
        "const __names={};",
        start["names"]
    )))
    .map_err(|e| e.to_string())?;
    let dispatch = cx
        .eval(Source::from_bytes(PRELUDE))
        .map_err(|e| e.to_string())?
        .as_object()
        .ok_or("Missing RPC dispatcher")?;
    let script = format!("(async function(){{'use strict';\n{source}\n}})()");
    let promise = cx
        .eval(Source::from_bytes(&script))
        .map_err(|e| e.to_string())?;
    let promise = JsPromise::from_object(promise.as_object().ok_or("Missing result promise")?)
        .map_err(|e| e.to_string())?;
    loop {
        cx.run_jobs().map_err(|e| e.to_string())?;
        match promise.state() {
            PromiseState::Fulfilled(value) => {
                if !pending.borrow().calls.is_empty() {
                    return Err(
                        "Unawaited tool calls: await every operation before returning".into(),
                    );
                }
                let encoded = value
                    .to_json(&mut cx)
                    .map_err(|e| e.to_string())?
                    .unwrap_or(Value::Null);
                if encoded.to_string().len() > 65536 {
                    return Err(
                        "Result exceeds 64KiB; filter or summarize tool data before returning"
                            .into(),
                    );
                }
                send(out, &json!({"type":"result","value":encoded}))?;
                return Ok(());
            }
            PromiseState::Rejected(error) => {
                let error = error
                    .to_string(&mut cx)
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_else(|_| "JavaScript rejected".into());
                return Err(error);
            }
            PromiseState::Pending => {}
        }
        let calls = std::mem::take(&mut pending.borrow_mut().calls);
        if calls.is_empty() {
            return Err("Promise did not settle; only tools can perform asynchronous work".into());
        }
        let count = calls.len();
        for call in calls {
            send(out, &call)?;
        }
        // Resolve the whole dispatched wave before running another JS microtask.
        // This gives the host a bounded parallel read wave and a write barrier.
        for _ in 0..count {
            let reply = receive(input)?;
            let reply = JsValue::from_json(&reply, &mut cx).map_err(|e| e.to_string())?;
            dispatch
                .call(&JsValue::undefined(), &[reply], &mut cx)
                .map_err(|e| e.to_string())?;
        }
    }
}
/// Called before any native UI startup, exclusively in a spawned worker.
pub fn main() {
    #[cfg(unix)]
    unsafe {
        let cpu = libc::rlimit {
            rlim_cur: 10,
            rlim_max: 10,
        };
        libc::setrlimit(libc::RLIMIT_CPU, &cpu);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        let memory = libc::rlimit {
            rlim_cur: 512 * 1024 * 1024,
            rlim_max: 512 * 1024 * 1024,
        };
        libc::setrlimit(libc::RLIMIT_AS, &memory);
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if let Err(error) = evaluate(&mut input, &mut out) {
        let _ = send(
            &mut out,
            &json!({"type":"error","error":error.chars().take(2048).collect::<String>()}),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn execute(code: &str, replies: &[Value]) -> Vec<Value> {
        let mut input = json!({"code":code,"names":["read_file","write_file"]}).to_string() + "\n";
        for reply in replies {
            input.push_str(&(reply.to_string() + "\n"));
        }
        let mut output = Vec::new();
        let result = evaluate(&mut std::io::Cursor::new(input), &mut output);
        if let Err(error) = result {
            let _ = send(&mut output, &json!({"type":"error","error":error}));
        }
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
    #[test]
    fn dependent_calls_and_filtered_return() {
        let frames = execute(
            "const f=await tools.read_file({path:'a'});await tools.write_file({path:'a',content:f.content+'!'});return {ok:true};",
            &[
                json!({"id":1,"success":true,"value":{"content":"hello"}}),
                json!({"id":2,"success":true,"value":"written"}),
            ],
        );
        assert_eq!(frames[1]["arguments"]["content"], "hello!");
        assert_eq!(frames.last().unwrap()["value"], json!({"ok":true}));
        assert!(!frames.last().unwrap().to_string().contains("hello"));
    }
    #[test]
    fn parallel_wave_control_flow_and_absent_node_apis() {
        let frames = execute(
            "const values=await Promise.all([tools.read_file({path:'a'}),tools.read_file({path:'b'})]);return values.map(x=>x.n).filter(n=>n>1);",
            &[
                json!({"id":2,"success":true,"value":{"n":2}}),
                json!({"id":1,"success":true,"value":{"n":1}}),
            ],
        );
        assert_eq!(frames[0]["type"], "call");
        assert_eq!(frames[1]["type"], "call");
        assert_eq!(frames[2]["value"], json!([2]));
        let frames = execute("return [typeof process,typeof require,typeof fetch];", &[]);
        assert_eq!(
            frames[0]["value"],
            json!(["undefined", "undefined", "undefined"])
        );
    }
    #[test]
    fn dynamic_import_cannot_read_an_existing_local_module() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.mjs");
        std::fs::write(&path, "export const secret='private';").unwrap();
        let code = format!(
            "const m=await import({}); return m.secret;",
            serde_json::to_string(path.to_str().unwrap()).unwrap()
        );
        let result = execute(&code, &[]);
        assert_eq!(result.last().unwrap()["type"], "error");
        assert!(!result.last().unwrap().to_string().contains("private"));
    }
    #[test]
    fn rejects_infinite_loop_oversized_return_and_unawaited_tools() {
        for code in [
            "while(true){}",
            "return 'x'.repeat(70000);",
            "tools.write_file({path:'a',content:'x'});return true;",
        ] {
            assert_eq!(execute(code, &[]).last().unwrap()["type"], "error");
        }
    }
}
