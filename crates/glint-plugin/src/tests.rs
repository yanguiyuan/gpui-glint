use super::*;
use serde_json::json;
use std::collections::BTreeSet;
fn plugin(source: &str, capabilities: &[&str]) -> Plugin {
    Plugin::from_source(&json!({"id":"test.plugin","name":"Test","version":"1.0.0","api_version":1,"entry":"main.js","capabilities":capabilities,"hooks":["database.inspect"]}).to_string(), source).unwrap()
}
#[test]
fn database_host_api_is_independent_of_http_and_gpui() {
    let plugin = plugin(
        "glint.on('database.inspect', input => glint.host.call('database.describe', input.table));",
        &["database.metadata"],
    );
    let mut registry = HostRegistry::default();
    registry
        .register("database.describe", "database.metadata", |args| {
            Ok(json!({"name":args[0],"columns":["id","title"]}))
        })
        .unwrap();
    let mut runtime = PluginRuntime::activate(
        &plugin,
        registry,
        plugin.manifest.capabilities.clone(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .invoke("database.inspect", &json!({"table":"notes"}))
            .unwrap(),
        json!({"name":"notes","columns":["id","title"]})
    );
}
#[test]
fn host_authority_cannot_be_gained_by_forging_javascript() {
    let p = plugin(
        "glint.on('database.inspect', () => __glintHost('database.delete', '[]'));",
        &[],
    );
    let mut registry = HostRegistry::default();
    registry
        .register("database.delete", "database.write", |_| {
            panic!("must never reach host")
        })
        .unwrap();
    let mut runtime = PluginRuntime::activate(
        &p,
        registry,
        BTreeSet::from(["database.write".into()]),
        Limits::default(),
    )
    .unwrap();
    assert!(runtime.invoke("database.inspect", &json!({})).is_err());
}
#[test]
fn missing_grants_fail_before_source_executes() {
    let p = plugin("throw new Error('secret');", &["database.write"]);
    let error = PluginRuntime::activate(
        &p,
        HostRegistry::default(),
        BTreeSet::new(),
        Limits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.0, "Plugin permissions were not granted");
}
#[test]
fn thrown_secret_is_not_in_diagnostic() {
    let p = plugin(
        "glint.on('database.inspect', () => {throw new Error('app-key-private')});",
        &[],
    );
    let mut runtime = PluginRuntime::activate(
        &p,
        HostRegistry::default(),
        BTreeSet::new(),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .invoke("database.inspect", &json!({}))
            .unwrap_err()
            .0,
        "Plugin hook failed"
    );
}
#[test]
fn source_limits_and_manifest_version_are_checked() {
    let mut p = plugin("", &[]);
    p.manifest.api_version = 2;
    assert!(p.manifest.validate().is_err());
    p.manifest.api_version = 1;
    p.manifest.entry = "../secret.js".into();
    assert!(p.manifest.validate().is_err());
    assert!(Plugin::from_source("{}", &"x".repeat(256 * 1024 + 1)).is_err());
}
#[test]
fn synchronous_hooks_reject_promises_and_undeclared_hooks() {
    let p = plugin("glint.on('database.inspect', async () => 1);", &[]);
    let mut runtime = PluginRuntime::activate(
        &p,
        HostRegistry::default(),
        BTreeSet::new(),
        Limits::default(),
    )
    .unwrap();
    assert!(runtime.invoke("database.inspect", &json!(null)).is_err());
    assert!(runtime.invoke("database.delete", &json!(null)).is_err());
}
#[test]
fn plugin_calls_do_not_share_javascript_state() {
    let p = plugin(
        "let count = 0; glint.on('database.inspect', () => ++count);",
        &[],
    );
    for _ in 0..2 {
        let mut runtime = PluginRuntime::activate(
            &p,
            HostRegistry::default(),
            BTreeSet::new(),
            Limits::default(),
        )
        .unwrap();
        assert_eq!(
            runtime.invoke("database.inspect", &json!(null)).unwrap(),
            json!(1)
        );
    }
}
#[test]
fn symlink_entry_cannot_escape_plugin_directory() {
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let p = plugin("", &[]);
        std::fs::write(
            dir.path().join("glint-plugin.json"),
            serde_json::to_vec(&p.manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(outside.path().join("main.js"), "").unwrap();
        std::os::unix::fs::symlink(outside.path().join("main.js"), dir.path().join("main.js"))
            .unwrap();
        assert!(Plugin::load(dir.path()).is_err());
    }
}

#[test]
fn page_contract_rejects_duplicate_controls_and_deep_layouts() {
    let action = Node::Button {
        id: "same".into(),
        label: "Refresh".into(),
        action: "refresh".into(),
        data: json!(null),
    };
    assert!(
        Page {
            title: "Test".into(),
            body: Node::Row {
                children: vec![action.clone(), action]
            }
        }
        .validate()
        .is_err()
    );
    let mut node = Node::Text {
        text: "Hello".into(),
    };
    for _ in 0..18 {
        node = Node::Column {
            children: vec![node],
        };
    }
    assert!(
        Page {
            title: "Test".into(),
            body: node
        }
        .validate()
        .is_err()
    );
}

#[test]
fn approvals_are_bound_to_source_and_permissions() {
    let mut p = plugin("glint.on('database.inspect', () => 1);", &[]);
    let approval = Approval::for_plugin(&p);
    assert!(approval.matches(&p));
    p.source.push_str("\n// changed");
    assert!(
        PluginRuntime::activate_approved(&p, HostRegistry::default(), &approval, Limits::default())
            .is_err()
    );
    p.source.truncate(p.source.len() - 11);
    p.manifest.capabilities.insert("database.write".into());
    assert!(!approval.matches(&p));
}
