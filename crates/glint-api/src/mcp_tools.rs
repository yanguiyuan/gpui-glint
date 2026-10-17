//! Tool discovery and argument validation share the same definitions.
use serde_json::{Value, json};

fn string() -> Value {
    json!({"type":"string"})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn enumeration(values: &[&str]) -> Value {
    json!({"type":"string","enum":values})
}
fn pair() -> Value {
    object(
        json!({"key":string(),"value":string(),"enabled":boolean(),"note":string()}),
        &["key", "value"],
    )
}
fn array(items: Value) -> Value {
    json!({"type":"array","items":items})
}
pub fn draft_schema() -> Value {
    object(
        json!({
            "method":enumeration(&["GET","POST","PUT","PATCH","DELETE","HEAD","OPTIONS","WS"]),
            "url":string(),"body":string(),"body_format":enumeration(&["raw","url_encoded"]),
            "params":array(pair()),"headers":array(pair()),"cookies":array(pair()),"form":array(pair()),
            "oauth": {"anyOf": [object(json!({
                "id":string(),"authorization_url":string(),"token_url":string(),"client_id":string(),
                "redirect_uri":string(),"scope":string(),"pkce":boolean(),"basic_auth":boolean(),"manual_callback":boolean(),"rolling_refresh_expiry":boolean()
            }), &[]), {"type":"null"}]},
            "extractors":array(pair()),"auth":boolean(),"token":string(),"skip_tls_verification":boolean()
        }),
        &[],
    )
}
fn node() -> Value {
    object(
        json!({"request_id":string(),"collection":string(),"path":string()}),
        &[],
    )
}
pub fn catalog() -> Vec<Value> {
    let mut tools = vec![];
    let mut add = |name: &str,
                   description: &str,
                   properties: Value,
                   required: &[&str],
                   read_only: bool,
                   destructive: bool| {
        tools.push(json!({"name":format!("glint_{name}"),"description":description,
            "inputSchema":object(properties,required),
            "annotations":{"readOnlyHint":read_only,"destructiveHint":destructive,"openWorldHint":!read_only}}));
    };
    add(
        "mcp_history_list",
        "Read persisted MCP tool attempts with date/time, client, success/error and duration, including rejected calls. Newest first; use next_before_id for stable pagination.",
        json!({"query":string(),"outcome":enumeration(&["all","success","error"]),"before_id":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":200}}),
        &[],
        true,
        false,
    );
    add(
        "mcp_history_get",
        "Read one persisted MCP call's arguments/result. Audit values are always redacted and large values are bounded, even if the original call requested secrets.",
        json!({"id":{"type":"integer","minimum":1}}),
        &["id"],
        true,
        false,
    );
    let integer = json!({"type":"integer","minimum":0});
    add(
        "workspace_get",
        "Inspect the live GUI, collections, request IDs, environment names, operation state and errors. Does not switch requests.",
        json!({}),
        &[],
        true,
        false,
    );
    add(
        "workspace_export",
        "Export the live workspace. Sensitive fields are redacted unless include_secrets=true. Export wrapper contains workspace and redacted.",
        json!({"include_secrets":boolean()}),
        &[],
        true,
        false,
    );
    add(
        "workspace_import",
        "Replace the live workspace with a complete unredacted export after validation. Cancels active requests. Set replace=true explicitly.",
        json!({"workspace":{"type":"object"},"replace":boolean()}),
        &["workspace", "replace"],
        false,
        true,
    );
    add(
        "collection_create",
        "Create an empty collection.",
        json!({"name":string()}),
        &["name"],
        false,
        false,
    );
    add(
        "collection_rename",
        "Rename a collection and all its request references.",
        json!({"collection":string(),"name":string()}),
        &["collection", "name"],
        false,
        false,
    );
    add(
        "collection_delete",
        "Delete a collection; nonempty collections require delete_requests=true. Deletes contained request versions/history as well.",
        json!({"collection":string(),"delete_requests":boolean()}),
        &["collection"],
        false,
        true,
    );
    add(
        "directory_create",
        "Create a directory under parent (empty string means collection root).",
        json!({"collection":string(),"parent":string(),"name":string()}),
        &["collection", "name"],
        false,
        false,
    );
    add(
        "directory_rename",
        "Rename a directory, preserving descendants and request data.",
        json!({"collection":string(),"path":string(),"name":string()}),
        &["collection", "path", "name"],
        false,
        false,
    );
    add(
        "directory_delete",
        "Delete a directory and subdirectories; requests move to its parent.",
        json!({"collection":string(),"path":string()}),
        &["collection", "path"],
        false,
        true,
    );
    add(
        "directory_defaults_get",
        "Read explicit and inherited directory defaults. Empty path means collection root.",
        json!({"collection":string(),"path":string(),"include_secrets":boolean()}),
        &["collection"],
        true,
        false,
    );
    add(
        "directory_defaults_set",
        "Replace the selected inherited sections. Empty sections restores inheritance. Draft is merged over a blank draft; use plugin_bind for per-request plugin grants.",
        json!({"collection":string(),"path":string(),"sections":array(enumeration(&["endpoint","auth","params","headers","cookies","body","extractors","settings","plugins"])),"draft":draft_schema(),"plugin_bindings":array(object(json!({"identity":string(),"action_id":string(),"enabled":boolean()}), &["identity"]))}),
        &["collection", "sections", "draft"],
        false,
        false,
    );
    add(
        "tree_move",
        "Move/reorder a request or directory. Node is {request_id} or {collection,path}. Placement: inside/before/after. Collection roots can be reordered before/after.",
        json!({"source":node(),"target":node(),"placement":enumeration(&["inside","before","after"])}),
        &["source", "target", "placement"],
        false,
        false,
    );
    add(
        "request_create",
        "Create and select a request in a collection/directory, inheriting defaults, then apply an optional draft patch.",
        json!({"collection":string(),"path":string(),"name":string(),"draft":draft_schema()}),
        &["collection", "name"],
        false,
        false,
    );
    add(
        "request_get",
        "Read a saved/current draft by persistent request ID, including version/history summaries. Does not switch requests.",
        json!({"request_id":string(),"include_secrets":boolean()}),
        &["request_id"],
        true,
        false,
    );
    add(
        "request_update",
        "Patch a request draft/name. Array fields replace entire tables. An active running request must be cancelled first.",
        json!({"request_id":string(),"name":string(),"draft":draft_schema()}),
        &["request_id"],
        false,
        false,
    );
    add(
        "request_select",
        "Select/open a request in the GUI. Refuses to interrupt another running request; cancel it first.",
        json!({"request_id":string()}),
        &["request_id"],
        false,
        false,
    );
    add(
        "request_delete",
        "Delete a request and its versions/history using its persistent ID.",
        json!({"request_id":string()}),
        &["request_id"],
        false,
        true,
    );
    add(
        "request_send",
        "Start real HTTP/SSE or WS execution and return immediately. Optional environment selects by name. Poll request_status/response_read/websocket_read. One active execution per workbench.",
        json!({"request_id":string(),"environment":string()}),
        &["request_id"],
        false,
        false,
    );
    add(
        "request_cancel",
        "Cancel the active execution for this request; keep received SSE content. Disconnects WS.",
        json!({"request_id":string()}),
        &["request_id"],
        false,
        false,
    );
    add(
        "request_status",
        "Read running/connected state, HTTP status, byte count, notice and error. Does not wait for SSE/WS EOF.",
        json!({"request_id":string()}),
        &["request_id"],
        true,
        false,
    );
    add(
        "request_preview",
        "Resolve and validate URL/headers/body without network. include_secrets=true is required to read raw generated cURL/preview.",
        json!({"request_id":string(),"include_secrets":boolean()}),
        &["request_id"],
        true,
        false,
    );
    add(
        "response_read",
        "Read a bounded UTF-8 response slice and headers. offset/next_offset are byte offsets in the current text snapshot; SSE retains the latest 1 MiB. Optional record_id reads historical response.",
        json!({"request_id":string(),"record_id":string(),"offset":integer,"limit":json!({"type":"integer","minimum":1,"maximum":1048576}),"include_secrets":boolean()}),
        &["request_id"],
        true,
        false,
    );
    add(
        "curl_export",
        "Read the exact cURL snapshot from the last sent HTTP request, including plugin headers. Requires include_secrets=true to reveal it.",
        json!({"request_id":string(),"include_secrets":boolean()}),
        &["request_id"],
        true,
        false,
    );
    add(
        "traces_read",
        "Read request execution traces and the last request preview.",
        json!({"request_id":string(),"include_secrets":boolean()}),
        &["request_id"],
        true,
        false,
    );
    add(
        "response_clear",
        "Clear the retained response/traces. Refuses while a request is running.",
        json!({"request_id":string()}),
        &["request_id"],
        false,
        true,
    );
    add(
        "curl_import",
        "Parse cURL into a new request without sending. Inherits no directory defaults.",
        json!({"collection":string(),"path":string(),"name":string(),"curl":string()}),
        &["collection", "name", "curl"],
        false,
        false,
    );
    add(
        "version_save",
        "Save a changed request draft as a new version; identical drafts reuse the current latest version. Optional tag labels it.",
        json!({"request_id":string(),"tag":string()}),
        &["request_id"],
        false,
        false,
    );
    for (name, description, read, destructive) in [
        (
            "record_get",
            "Read a version/history record by persistent record ID. Response body is obtained with response_read(record_id).",
            true,
            false,
        ),
        (
            "record_restore",
            "Restore a version draft or history exchange into the GUI; refuses while running.",
            false,
            false,
        ),
        (
            "record_tag",
            "Set a version tag, without restoring the draft.",
            false,
            false,
        ),
        (
            "record_delete",
            "Delete one saved version/history record by persistent ID.",
            false,
            true,
        ),
    ] {
        add(
            name,
            description,
            json!({"request_id":string(),"record_id":string(),"tag":string(),"include_secrets":boolean()}),
            if name == "record_tag" {
                &["request_id", "record_id", "tag"]
            } else {
                &["request_id", "record_id"]
            },
            read,
            destructive,
        );
    }
    add(
        "environment_get",
        "Read global variables, or collection variables when collection is provided, with the global proxy settings. Secret variables are redacted by default.",
        json!({"environment":string(),"collection":string(),"include_secrets":boolean()}),
        &["environment"],
        true,
        false,
    );
    add(
        "environment_create",
        "Create a named global environment.",
        json!({"name":string()}),
        &["name"],
        false,
        false,
    );
    add(
        "environment_rename",
        "Rename an environment, preserving variables and current selection.",
        json!({"environment":string(),"name":string()}),
        &["environment", "name"],
        false,
        false,
    );
    add(
        "environment_delete",
        "Delete an environment and variables. The built-in no-environment entry cannot be deleted.",
        json!({"environment":string()}),
        &["environment"],
        false,
        true,
    );
    add(
        "environment_select",
        "Select the global environment by name. Running requests retain their original snapshot.",
        json!({"environment":string()}),
        &["environment"],
        false,
        false,
    );
    add(
        "environment_variable_set",
        "Create/update a global variable, or collection variable when collection is provided, by name (or rename via variable_id). Refuses to overwrite unsaved GUI environment edits.",
        json!({"environment":string(),"collection":string(),"variable_id":integer,"name":string(),"value":string(),"secret":boolean()}),
        &["environment", "name", "value"],
        false,
        false,
    );
    add(
        "environment_variable_delete",
        "Delete one saved variable by name.",
        json!({"environment":string(),"collection":string(),"name":string()}),
        &["environment", "name"],
        false,
        true,
    );
    add(
        "environment_variables_replace",
        "Replace all saved variables atomically. Refuses to overwrite unsaved GUI environment edits.",
        json!({"environment":string(),"collection":string(),"variables":array(object(json!({"name":string(),"value":string(),"secret":boolean()}), &["name","value"]))}),
        &["environment", "variables"],
        false,
        true,
    );
    add(
        "environment_proxy_set",
        "Replace an environment's HTTP/HTTPS proxy and TLS skip policy.",
        json!({"environment":string(),"enabled":boolean(),"url":string(),"skip_tls_verification":boolean()}),
        &["environment", "enabled", "url"],
        false,
        false,
    );
    add(
        "environment_draft",
        "Save or discard the GUI environment editor's unsaved variables/proxy for the named environment.",
        json!({"environment":string(),"collection":string(),"action":enumeration(&["save","discard"])}),
        &["environment", "action"],
        false,
        false,
    );
    add(
        "websocket_send",
        "Send a text message on the active WS connection. Templates resolve by default; set resolve_templates=false for literal text.",
        json!({"request_id":string(),"text":string(),"resolve_templates":boolean()}),
        &["request_id", "text"],
        false,
        false,
    );
    add(
        "websocket_read",
        "Read current WS message log and connected state. Log is bounded, cleared on request switch. Optional after filters by monotonically increasing message sequence.",
        json!({"request_id":string(),"after":integer,"include_secrets":boolean()}),
        &["request_id"],
        true,
        false,
    );
    add(
        "websocket_clear",
        "Clear the active WS message log.",
        json!({"request_id":string()}),
        &["request_id"],
        false,
        true,
    );
    add(
        "plugins_list",
        "List installed/bundled plugin manifests, identities, actions, grants and registry status.",
        json!({}),
        &[],
        true,
        false,
    );
    add(
        "plugin_bind",
        "Grant/revoke an installed plugin action on a request. identity comes from plugins_list. enabled=false disables it; remove=true removes the binding. Grants are tied to exact installed code fingerprint.",
        json!({"request_id":string(),"identity":string(),"action_id":string(),"enabled":boolean(),"remove":boolean()}),
        &["request_id", "identity"],
        false,
        false,
    );
    add(
        "plugin_bindings_reorder",
        "Replace the order of existing before-send action bindings. Provide each binding index exactly once.",
        json!({"request_id":string(),"order":array(integer.clone())}),
        &["request_id", "order"],
        false,
        false,
    );
    add(
        "registry_source_add",
        "Configure a signed plugin source with a trusted public key. Fetching occurs with registry_refresh.",
        json!({"name":string(),"url":string(),"public_key":string(),"allow_http":boolean()}),
        &["name", "url", "public_key"],
        false,
        false,
    );
    add(
        "registry_source_remove",
        "Remove a registry source; installed versions remain.",
        json!({"name":string()}),
        &["name"],
        false,
        true,
    );
    add(
        "registry_refresh",
        "Start signed registry refresh; poll plugins_list for completion/errors/releases.",
        json!({}),
        &[],
        false,
        false,
    );
    add(
        "plugin_install",
        "Start download and signature/hash validation of a remote plugin version. Poll plugins_list. Installation does not grant execution.",
        json!({"source":string(),"plugin_id":string(),"version":string()}),
        &["source", "plugin_id", "version"],
        false,
        false,
    );
    add(
        "plugin_uninstall",
        "Uninstall a remote version. Existing request bindings stay and fail validation until rebound.",
        json!({"source":string(),"plugin_id":string(),"version":string()}),
        &["source", "plugin_id", "version"],
        false,
        true,
    );
    add(
        "ui_set",
        "Set GUI theme, editor/response tabs, sidebar visibility, or open a workbench section.",
        json!({"theme":enumeration(&["light","dark"]),"section":enumeration(&["requests","environments","plugins","mcp"]),"config_tab":json!({"type":"integer","minimum":0,"maximum":6}),"response_tab":json!({"type":"integer","minimum":0,"maximum":4}),"show_activity":boolean(),"activity":json!({"type":"integer","minimum":0,"maximum":1}),"filter":string()}),
        &[],
        false,
        false,
    );
    add(
        "update_check",
        "Start application update check; poll update_status.",
        json!({}),
        &[],
        false,
        false,
    );
    add(
        "update_status",
        "Read application update state, version, download progress and errors.",
        json!({}),
        &[],
        true,
        false,
    );
    add(
        "update_download",
        "Download the available update using the existing updater. Available on supported installed Windows builds.",
        json!({}),
        &[],
        false,
        false,
    );
    add(
        "update_cancel",
        "Cancel application update checking/downloading.",
        json!({}),
        &[],
        false,
        false,
    );
    add(
        "update_install",
        "Install a downloaded app update and restart using existing unsaved-work/running-request guards.",
        json!({}),
        &[],
        false,
        true,
    );
    tools
}

pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    if let Some(choices) = schema["anyOf"].as_array() {
        return if choices.iter().any(|choice| validate(choice, value).is_ok()) {
            Ok(())
        } else {
            Err("参数不符合允许的数据结构".into())
        };
    }
    let valid_type = match schema["type"].as_str() {
        Some("null") => value.is_null(),
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.as_u64().is_some(),
        _ => true,
    };
    if !valid_type {
        return Err(format!("参数类型应为 {}", schema["type"]));
    }
    if let Some(options) = schema["enum"].as_array() {
        if !options.contains(value) {
            return Err(format!("参数不在可选值中：{value}"));
        }
    }
    if let Some(number) = value.as_u64() {
        if schema["minimum"].as_u64().is_some_and(|n| number < n)
            || schema["maximum"].as_u64().is_some_and(|n| number > n)
        {
            return Err("数值超出允许范围".into());
        }
    }
    if let Some(fields) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if !fields.contains_key(key) {
                    return Err(format!("缺少参数：{key}"));
                }
            }
        }
        for (key, value) in fields {
            if let Some(child) = schema["properties"].get(key) {
                validate(child, value).map_err(|e| format!("{key}: {e}"))?;
            } else if schema["additionalProperties"] == false {
                return Err(format!("未知参数：{key}"));
            }
        }
    }
    if let Some(items) = value.as_array() {
        for item in items {
            validate(&schema["items"], item)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_catalog_names_schemas_and_annotations_are_consistent() {
        let tools = catalog();
        let mut names = std::collections::HashSet::new();
        assert!(tools.len() >= 50);
        for tool in &tools {
            assert!(names.insert(tool["name"].as_str().unwrap()));
            assert!(tool["description"].as_str().unwrap().len() > 20);
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
            assert!(tool["annotations"]["readOnlyHint"].is_boolean());
        }
        let schema = &tools
            .iter()
            .find(|t| t["name"] == "glint_request_create")
            .unwrap()["inputSchema"];
        assert!(validate(schema,&json!({"collection":"API","name":"Echo","draft":{"method":"WS","headers":[{"key":"Origin","value":"http://localhost"}]}})).is_ok());
        for arguments in [
            json!({}),
            json!({"collection":"API","name":3}),
            json!({"collection":"API","name":"x","typo":true}),
            json!({"collection":"API","name":"x","draft":{"method":"TYPO"}}),
        ] {
            assert!(validate(schema, &arguments).is_err());
        }
        let schema = &tools
            .iter()
            .find(|t| t["name"] == "glint_response_read")
            .unwrap()["inputSchema"];
        assert!(validate(schema, &json!({"request_id":"id","offset":-1})).is_err());
        assert!(validate(schema, &json!({"request_id":"id","limit":1048577})).is_err());
    }
}
