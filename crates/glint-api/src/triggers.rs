//! Conditional request dependencies, evaluated before resolving the main request.
use super::{
    environment::{EnvironmentVariables, ProxySettings},
    http,
    model::{Draft, Request, TriggerAction, TriggerCondition},
    plugins, post_actions,
};

pub fn needs_refresh(
    name: &str,
    local: &EnvironmentVariables,
    global: &EnvironmentVariables,
    now: i64,
) -> bool {
    local
        .variables
        .iter()
        .find(|v| v.name == name)
        .or_else(|| global.variables.iter().find(|v| v.name == name))
        .is_none_or(|v| {
            v.value.trim().is_empty() || v.expires_at.is_some_and(|expiry| expiry <= now)
        })
}

pub struct TriggerContext {
    pub requests: Vec<Request>,
    pub collection: String,
    pub global: EnvironmentVariables,
    pub local: EnvironmentVariables,
    pub proxy: ProxySettings,
    pub catalog: Vec<gpui_glint::plugins::Plugin>,
    pub completed: Vec<(String, EnvironmentVariables, Vec<String>)>,
}
impl TriggerContext {
    pub fn run<'a>(
        &'a mut self,
        draft: &'a Draft,
        stack: Vec<String>,
    ) -> futures_util::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if stack.len() > 32 {
                return Err("变量刷新依赖超过 32 层".into());
            }
            for trigger in draft.triggers.iter().filter(|r| r.enabled) {
                if !trigger.condition.evaluate(
                    &self.local,
                    &self.global,
                    chrono::Utc::now().timestamp(),
                )? {
                    continue;
                }
                self.execute_action(&trigger.action, stack.clone()).await?;
            }
            Ok(())
        })
    }
    fn execute_action<'a>(
        &'a mut self,
        action: &'a TriggerAction,
        mut stack: Vec<String>,
    ) -> futures_util::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            match action {
                TriggerAction::RunRequest { request_id } => {
                    let target = self
                        .requests
                        .iter()
                        .find(|r| r.id == *request_id && r.collection == self.collection)
                        .cloned()
                        .ok_or_else(|| "触发器的目标请求不存在或不属于当前集合".to_owned())?;
                    if stack.contains(&target.id) {
                        return Err("触发器请求存在循环依赖".to_owned());
                    }
                    if target.draft.method == "WS" || target.draft.oauth.is_some() {
                        return Err(
                            "刷新请求须为普通 HTTP 请求，暂不支持 OAuth 自动授权或 WebSocket"
                                .into(),
                        );
                    }
                    stack.push(target.id.clone());
                    self.run(&target.draft, stack.clone()).await?;
                    stack.pop();
                    let mut values = self.global.values();
                    values.0.extend(self.local.values().0);
                    let request = http::PreparedRequest::new(&target.draft, &values)?
                        .with_proxy(&self.proxy)?;
                    plugins::validate_variables(&target.draft.plugins, &self.catalog, &values)?;
                    let request =
                        plugins::apply(request, &target.draft.plugins, &self.catalog, &values)?;
                    let response = http::execute_stream(request, None).await?;
                    if !(200..300).contains(&response.status) || response.is_event_stream() {
                        return Err(format!(
                            "刷新请求 {} 失败：HTTP {}",
                            target.name, response.status
                        ));
                    }
                    // A local override must inherit a globally sensitive variable's secrecy.
                    for rule in target.draft.extractors.iter().filter(|r| r.enabled) {
                        if !self
                            .local
                            .variables
                            .iter()
                            .any(|v| v.name == rule.key.trim())
                        {
                            if let Some(global) = self
                                .global
                                .variables
                                .iter()
                                .find(|v| v.name == rule.key.trim() && v.secret)
                            {
                                self.local.save(None, &global.name, "", true)?;
                            }
                        }
                    }
                    let (local, names) = if target.draft.extractors.iter().any(|r| r.enabled) {
                        post_actions::extract(
                            &target.draft.extractors,
                            response.json.as_ref(),
                            Some(&self.local),
                        )?
                    } else {
                        (self.local.clone(), vec![])
                    };
                    self.local = local;
                    self.completed
                        .push((target.name, self.local.clone(), names));
                }
            }
            Ok(())
        })
    }
}

impl TriggerCondition {
    pub fn evaluate(
        &self,
        local: &EnvironmentVariables,
        global: &EnvironmentVariables,
        now: i64,
    ) -> Result<bool, String> {
        match self {
            Self::VariableExpired { variable } => {
                if variable.trim().is_empty() {
                    return Err("变量过期条件缺少变量名".into());
                }
                Ok(needs_refresh(variable.trim(), local, global, now))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pair, Trigger};

    fn trigger(variable: &str, request: &str) -> Trigger {
        Trigger {
            enabled: true,
            condition: TriggerCondition::VariableExpired {
                variable: variable.into(),
            },
            action: TriggerAction::RunRequest {
                request_id: request.into(),
            },
        }
    }
    fn context() -> TriggerContext {
        TriggerContext {
            requests: vec![],
            collection: "API".into(),
            global: Default::default(),
            local: Default::default(),
            proxy: Default::default(),
            catalog: vec![],
            completed: vec![],
        }
    }
    fn request(id: &str, draft: Draft) -> Request {
        Request {
            id: id.into(),
            tree_order: 0,
            name: id.into(),
            collection: "API".into(),
            group: "".into(),
            draft,
            versions: vec![],
            history: vec![],
        }
    }

    #[test]
    fn condition_respects_scope_expiration_empty_values_and_legacy_variables() {
        let mut ctx = context();
        let condition = TriggerCondition::VariableExpired {
            variable: "token".into(),
        };
        assert!(condition.evaluate(&ctx.local, &ctx.global, 100).unwrap());
        ctx.global.save(None, "token", "global", false).unwrap();
        assert!(!condition.evaluate(&ctx.local, &ctx.global, 100).unwrap());
        ctx.local.save(None, "token", "local", false).unwrap();
        ctx.local.variables[0].expires_at = Some(100);
        assert!(condition.evaluate(&ctx.local, &ctx.global, 100).unwrap());
        assert!(!condition.evaluate(&ctx.local, &ctx.global, 99).unwrap());
        ctx.local.variables[0].value.clear();
        assert!(condition.evaluate(&ctx.local, &ctx.global, 99).unwrap());
    }

    #[test]
    fn actions_are_independent_serializable_and_cycles_or_foreign_targets_fail() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = Draft {
            triggers: vec![trigger("token", "refresh")],
            ..Default::default()
        };
        let value = serde_json::to_value(&root).unwrap();
        assert_eq!(
            value["triggers"][0]["condition"]["kind"],
            "variable_expired"
        );
        assert_eq!(value["triggers"][0]["action"]["kind"], "run_request");
        let restored: Draft = serde_json::from_value(value).unwrap();
        assert_eq!(restored, root);
        let mut ctx = context();
        ctx.requests.push(request(
            "refresh",
            Draft {
                triggers: vec![trigger("other", "root")],
                ..Default::default()
            },
        ));
        ctx.requests.push(request("root", root.clone()));
        assert!(
            rt.block_on(ctx.run(&root, vec!["root".into()]))
                .unwrap_err()
                .contains("循环")
        );
        ctx.requests[0].collection = "Other".into();
        assert!(
            rt.block_on(ctx.run(&root, vec!["root".into()]))
                .unwrap_err()
                .contains("不属于")
        );
        let mut disabled = root;
        disabled.triggers[0].enabled = false;
        assert!(rt.block_on(ctx.run(&disabled, vec!["root".into()])).is_ok());
        assert!(ctx.completed.is_empty());
    }

    #[test]
    fn request_action_extracts_token_before_main_resolution_and_skips_when_fresh() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0; 4096];
            socket.read(&mut buffer).unwrap();
            let body = r#"{"access_token":"fresh-token","expires_in":7199}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let mut extractor = Pair::new("access_token", "$.access_token", "");
        extractor.expires_in = "$.expires_in".into();
        let mut ctx = context();
        ctx.global.save(None, "access_token", "", true).unwrap();
        ctx.requests.push(request(
            "login",
            Draft {
                url: url.clone(),
                extractors: vec![extractor],
                ..Default::default()
            },
        ));
        let root = Draft {
            url,
            auth: true,
            token: "{{access_token}}".into(),
            triggers: vec![
                trigger("access_token", "login"),
                trigger("access_token", "login"),
            ],
            ..Default::default()
        };
        assert!(http::PreparedRequest::new(&root, &ctx.global.values()).is_err());
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(ctx.run(&root, vec!["root".into()]))
            .unwrap();
        server.join().unwrap();
        assert_eq!(ctx.completed.len(), 1);
        assert_eq!(ctx.local.variables[0].value, "fresh-token");
        assert!(ctx.local.variables[0].secret);
        assert!(ctx.local.variables[0].expires_at.unwrap() > chrono::Utc::now().timestamp());
        let mut values = ctx.global.values();
        values.0.extend(ctx.local.values().0);
        assert!(http::PreparedRequest::new(&root, &values).is_ok());
        assert!(ctx.global.variables[0].value.is_empty());
    }
}
