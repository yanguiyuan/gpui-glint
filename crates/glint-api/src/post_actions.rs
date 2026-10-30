use super::{environment::EnvironmentVariables, model::Pair};
use serde_json::Value;
use serde_json_path::JsonPath;
use std::collections::HashSet;

/// Stage every write so failed rules cannot leave a partially updated environment.
pub fn extract(
    rules: &[Pair],
    json: Option<&Value>,
    environment: Option<&EnvironmentVariables>,
) -> Result<(EnvironmentVariables, Vec<String>), String> {
    let mut staged = environment.cloned().ok_or("请先选择环境")?;
    let json = json.ok_or("响应体不是有效 JSON")?;
    let mut names = vec![];
    let mut seen = HashSet::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        let name = rule.key.trim();
        if !seen.insert(name) {
            return Err(format!("变量 {name} 存在重复提取规则"));
        }
        let path = JsonPath::parse(rule.value.trim())
            .map_err(|_| format!("变量 {name} 的 JSONPath 无效"))?;
        let nodes = path.query(json).all();
        let value = match nodes.as_slice() {
            [] => return Err(format!("变量 {name} 的 JSONPath 未匹配到数据")),
            [Value::String(value)] => value.clone(),
            [value] => value.to_string(),
            values => serde_json::to_string(values).map_err(|_| "无法序列化提取结果")?,
        };
        let existing = staged.variables.iter().find(|v| v.name == name);
        let id = existing.map(|v| v.id);
        let secret = existing.is_some_and(|v| v.secret);
        let expires_at = if rule.expires_in.trim().is_empty() {
            None
        } else {
            let path = JsonPath::parse(rule.expires_in.trim())
                .map_err(|_| format!("变量 {name} 的有效期 JSONPath 无效"))?;
            let nodes = path.query(json).all();
            let seconds = match nodes.as_slice() {
                [Value::Number(n)] => n.as_u64(),
                [Value::String(s)] => s.parse::<u64>().ok(),
                _ => None,
            }
            .ok_or_else(|| format!("变量 {name} 的有效期须匹配一个非负整数（秒）"))?;
            Some(
                i64::try_from(seconds)
                    .ok()
                    .and_then(|s| chrono::Utc::now().timestamp().checked_add(s))
                    .ok_or_else(|| format!("变量 {name} 的有效期超出范围"))?,
            )
        };
        let id = staged.save(id, name, &value, secret)?;
        staged
            .variables
            .iter_mut()
            .find(|v| v.id == id)
            .unwrap()
            .expires_at = expires_at;
        names.push(name.to_owned());
    }
    Ok((staged, names))
}

impl super::Workbench {
    pub(super) fn apply_extractors_for_id(
        &mut self,
        draft: &super::model::Draft,
        environment_index: usize,
        collection_id: &str,
        response: &super::http::HttpResponse,
        cx: &mut gpui::Context<Self>,
    ) -> Result<usize, String> {
        if !draft.extractors.iter().any(|r| r.enabled) {
            return Ok(0);
        }
        let collection = self
            .workspace
            .collections
            .iter()
            .find(|c| c.id == collection_id)
            .map(|c| c.collection.clone())
            .ok_or("提取变量的所属集合已不存在")?;
        self.apply_extractors(draft, environment_index, &collection, response, cx)
    }
    pub(super) fn apply_extractors(
        &mut self,
        draft: &super::model::Draft,
        environment_index: usize,
        collection_name: &str,
        response: &super::http::HttpResponse,
        cx: &mut gpui::Context<Self>,
    ) -> Result<usize, String> {
        if !draft.extractors.iter().any(|rule| rule.enabled) {
            return Ok(0);
        }
        let collection = self
            .workspace
            .collections
            .iter()
            .position(|c| c.collection == collection_name)
            .ok_or("提取变量的所属集合已不存在")?;
        let environment_name = self.environment_name(environment_index).to_owned();
        let mut local = self.workspace.collections[collection]
            .environments
            .get(&environment_name)
            .cloned()
            .unwrap_or_default();
        // Preserve a globally marked secret when a rule first creates its local override.
        for rule in draft.extractors.iter().filter(|r| r.enabled) {
            if !local.variables.iter().any(|v| v.name == rule.key.trim()) {
                if let Some(global) = self.workspace.environment_inputs[environment_index]
                    .variables
                    .iter()
                    .find(|v| v.name == rule.key.trim() && v.secret)
                {
                    local.save(None, &global.name, "", true)?;
                }
            }
        }
        let result = extract(&draft.extractors, response.json.as_ref(), Some(&local));
        match result {
            Ok((environment, names)) => {
                self.workspace.collections[collection]
                    .environments
                    .insert(environment_name, environment);
                self.merge_extracted_variables(environment_index, collection_name, &names, cx);
                self.sync_reference_inputs(cx);
                self.push_trace(
                    format!(
                        "后置动作完成 · 已写入 {} 个变量到集合 {} · {}",
                        names.len(),
                        collection_name,
                        self.environment_name(environment_index)
                    ),
                    cx,
                );
                Ok(names.len())
            }
            Err(error) => {
                self.push_trace(format!("后置动作失败：{error}"), cx);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expiry_is_extracted_atomically_and_old_data_remains_compatible() {
        let mut rule = Pair::new("access_token", "$.access_token", "");
        rule.expires_in = "$.expires_in".into();
        let mut env = EnvironmentVariables::default();
        env.save(None, "access_token", "old", true).unwrap();
        let before = chrono::Utc::now().timestamp();
        let (updated, _) = extract(
            &[rule.clone()],
            Some(&json!({"access_token":"new","expires_in":7199})),
            Some(&env),
        )
        .unwrap();
        let token = &updated.variables[0];
        assert!(token.secret);
        assert!(
            (before + 7199..=chrono::Utc::now().timestamp() + 7199)
                .contains(&token.expires_at.unwrap())
        );
        let restored: EnvironmentVariables =
            serde_json::from_value(serde_json::to_value(&updated).unwrap()).unwrap();
        assert_eq!(restored.variables, updated.variables);
        let mut edited = restored.clone();
        edited
            .replace(&[("access_token".into(), "new".into(), true)])
            .unwrap();
        assert_eq!(edited.variables[0].expires_at, token.expires_at);
        edited
            .replace(&[("access_token".into(), "manual".into(), true)])
            .unwrap();
        assert_eq!(edited.variables[0].expires_at, None);
        for invalid in [
            json!(-1),
            json!(1.5),
            json!(null),
            json!([]),
            json!("bad"),
            json!(u64::MAX),
        ] {
            assert!(
                extract(
                    &[rule.clone()],
                    Some(&json!({"access_token":"new", "expires_in":invalid})),
                    Some(&env)
                )
                .is_err()
            );
            assert_eq!(env.variables[0].value, "old");
        }
        let (updated, _) = extract(
            &[rule.clone()],
            Some(&json!({"access_token":"new","expires_in":"0"})),
            Some(&env),
        )
        .unwrap();
        assert!(updated.variables[0].expires_at.unwrap() <= chrono::Utc::now().timestamp());
        rule.expires_in.clear();
        let (updated, _) = extract(
            &[rule],
            Some(&json!({"access_token":"new"})),
            Some(&updated),
        )
        .unwrap();
        assert_eq!(updated.variables[0].expires_at, None);
        let old: super::super::environment::Variable =
            serde_json::from_value(json!({"id":0,"name":"token","value":"old","secret":true}))
                .unwrap();
        assert_eq!(old.expires_at, None);
    }

    #[gpui::test]
    fn post_actions_tab_edits_extracts_and_persists(cx: &mut gpui::TestAppContext) {
        use super::super::*;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            for (kind, name) in [
                (GroupEdit::CreateCollection, "API"),
                (GroupEdit::CreateEnvironment, "Development"),
            ] {
                host.group_edit = Some(kind);
                host.group_name
                    .update(cx, |entry, cx| entry.set_value(name, cx));
                host.save_group(cx);
            }
            host.add_request(cx);
            host.environment
                .update(cx, |select, cx| select.set_selected(Some("1".into()), cx));
            host
        });
        let bounds = window.debug_bounds("request-config-6").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.config, 6);
            host.extractors_table
                .update(cx, |_, cx| cx.emit(TableEvent::AddRow));
        });
        host.update(window, |host, cx| {
            assert_eq!(host.extractors.len(), 1);
            host.extractors[0]
                .key
                .update(cx, |entry, cx| entry.set_value("token", cx));
            host.extractors[0]
                .value
                .update(cx, |entry, cx| entry.set_value("$.token", cx));
            host.save(cx);
            let draft = host.snapshot(cx);
            let response = HttpResponse {
                status: 200,
                headers: vec![],
                text: r#"{"token":"abc"}"#.into(),
                bytes: 15,
                json: Some(json!({"token":"abc"})),
            };
            // The send-time environment wins even if the selector changes before completion.
            host.environment
                .update(cx, |select, cx| select.set_selected(Some("0".into()), cx));
            assert_eq!(
                host.apply_extractors(&draft, 1, "API", &response, cx)
                    .unwrap(),
                1
            );
            assert!(host.workspace.environment_inputs[0].variables.is_empty());
            assert_eq!(host.environment_values(1, Some("API")).0["token"], "abc");
            assert!(host.workspace.environment_inputs[1].variables.is_empty());
            assert!(host.variable_rows[1].is_empty());
            assert!(
                !host.workspace.traces[0]
                    .last()
                    .unwrap()
                    .message
                    .contains("abc")
            );
            host.environment
                .update(cx, |select, cx| select.set_selected(Some("1".into()), cx));
            host.add_request(cx);
            assert!(host.snapshot(cx).extractors.is_empty());
            host.choose(0, cx);
            assert_eq!(host.snapshot(cx).extractors, draft.extractors);
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert_eq!(host.snapshot(cx).extractors[0].value, "$.token");
            assert_eq!(host.environment_values(1, Some("API")).0["token"], "abc");
            assert!(host.workspace.environment_inputs[1].variables.is_empty());
        });
    }

    #[test]
    fn extracts_types_multiple_nodes_and_preserves_secret_identity() {
        let mut env = EnvironmentVariables::default();
        let id = env.save(None, "token", "old", true).unwrap();
        let rules = vec![
            Pair::new("token", "$.data.token", ""),
            Pair::new("ids", "$.items[*].id", ""),
            Pair::new("count", "$.count", ""),
            Pair::new("object", "$.data", ""),
            Pair::new("nothing", "$.nothing", ""),
            Pair::new("filtered", "$.items[?@.id > 1].id", ""),
        ];
        let body =
            json!({"data":{"token":"new"},"items":[{"id":1},{"id":2}],"count":3,"nothing":null});
        let (updated, names) = extract(&rules, Some(&body), Some(&env)).unwrap();
        assert_eq!(names.len(), 6);
        let token = &updated.variables[0];
        assert_eq!(
            (token.id, token.secret, token.value.as_str()),
            (id, true, "new")
        );
        let values = updated.values().0;
        assert_eq!(values["ids"], "[1,2]");
        assert_eq!(values["count"], "3");
        assert_eq!(values["object"], r#"{"token":"new"}"#);
        assert_eq!(values["nothing"], "null");
        assert_eq!(values["filtered"], "2");
        assert_eq!(env.variables[0].value, "old");
    }

    #[test]
    fn failures_are_atomic_and_disabled_rules_are_skipped() {
        let mut env = EnvironmentVariables::default();
        env.save(None, "token", "old", false).unwrap();
        let body = json!({"token":"new"});
        for bad in [
            Pair::new("missing", "$.missing", ""),
            Pair::new("bad", "$[", ""),
            Pair::new("1bad", "$", ""),
            Pair::new("token", "$", ""),
        ] {
            assert!(
                extract(
                    &[Pair::new("token", "$.token", ""), bad],
                    Some(&body),
                    Some(&env)
                )
                .is_err()
            );
            assert_eq!(env.variables[0].value, "old");
        }
        let mut disabled = Pair::new("bad", "$[", "");
        disabled.enabled = false;
        assert!(
            extract(&[disabled], Some(&body), Some(&env))
                .unwrap()
                .1
                .is_empty()
        );
        assert!(extract(&[], None, Some(&env)).is_err());
        assert!(extract(&[], Some(&body), None).is_err());
    }

    #[test]
    fn old_drafts_default_to_no_extractors_and_rules_round_trip() {
        let mut value = serde_json::to_value(super::super::model::Draft::default()).unwrap();
        value.as_object_mut().unwrap().remove("extractors");
        let mut draft: super::super::model::Draft = serde_json::from_value(value).unwrap();
        assert!(draft.extractors.is_empty());
        draft.extractors.push(Pair::new("token", "$.token", ""));
        let restored = serde_json::from_str::<super::super::model::Draft>(
            &serde_json::to_string(&draft).unwrap(),
        )
        .unwrap();
        assert_eq!(restored, draft);
    }
}
