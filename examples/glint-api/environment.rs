use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub id: u64,
    pub name: String,
    pub value: String,
    pub secret: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct EnvironmentVariables {
    pub variables: Vec<Variable>,
    next_id: u64,
}
impl EnvironmentVariables {
    /// Validate the complete table before replacing saved values, including renames.
    pub fn replace(&mut self, rows: &[(String, String, bool)]) -> Result<(), String> {
        let mut replacement = Self {
            variables: vec![],
            next_id: 0,
        };
        for (name, value, secret) in rows {
            replacement.save(None, name, value, *secret)?;
        }
        *self = replacement;
        Ok(())
    }
    #[cfg(test)]
    pub fn new(base_url: &str) -> Self {
        let mut environment = Self {
            variables: vec![],
            next_id: 0,
        };
        for (name, value, secret) in [
            ("base_url", base_url, false),
            ("workspace_id", "ws_personal", false),
            ("api_token", "", true),
        ] {
            environment.save(None, name, value, secret).unwrap();
        }
        environment
    }
    pub fn save(
        &mut self,
        id: Option<u64>,
        name: &str,
        value: &str,
        secret: bool,
    ) -> Result<u64, String> {
        let name = name.trim();
        let mut chars = name.chars();
        if !chars
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err("变量名须以字母或下划线开头，仅含字母、数字和下划线".into());
        }
        if self
            .variables
            .iter()
            .any(|variable| variable.name == name && Some(variable.id) != id)
        {
            return Err("当前环境中已存在同名变量".into());
        }
        if let Some(id) = id {
            let variable = self
                .variables
                .iter_mut()
                .find(|variable| variable.id == id)
                .ok_or("变量不存在")?;
            variable.name = name.into();
            variable.value = value.into();
            variable.secret = secret;
            return Ok(id);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.variables.push(Variable {
            id,
            name: name.into(),
            value: value.into(),
            secret,
        });
        Ok(id)
    }
    #[cfg(test)]
    pub fn remove(&mut self, id: u64) -> Option<Variable> {
        let index = self
            .variables
            .iter()
            .position(|variable| variable.id == id)?;
        Some(self.variables.remove(index))
    }
    pub fn values(&self) -> EnvironmentValues {
        EnvironmentValues(
            self.variables
                .iter()
                .map(|variable| (variable.name.clone(), variable.value.clone()))
                .collect(),
        )
    }
}
#[derive(Clone)]
pub struct EnvironmentValues(pub BTreeMap<String, String>);
impl EnvironmentValues {
    pub fn resolve(&self, input: &str) -> Result<String, String> {
        self.resolve_inner(input, &mut vec![])
    }
    fn resolve_inner(&self, input: &str, stack: &mut Vec<String>) -> Result<String, String> {
        let mut output = String::new();
        let mut rest = input;
        while let Some(start) = rest.find("{{") {
            output.push_str(&rest[..start]);
            let variable = &rest[start + 2..];
            let end = variable.find("}}").ok_or("环境变量缺少结束符 }}")?;
            let name = variable[..end].trim();
            let value = self
                .0
                .get(name)
                .ok_or_else(|| format!("未定义环境变量：{name}"))?;
            if value.is_empty() {
                return Err(format!("请在环境变量中填写 {name}"));
            }
            if stack.iter().any(|active| active == name) {
                return Err(format!("环境变量存在循环引用：{name}"));
            }
            if stack.len() >= 32 {
                return Err("环境变量嵌套超过 32 层".into());
            }
            stack.push(name.into());
            output.push_str(&self.resolve_inner(value, stack)?);
            stack.pop();
            rest = &variable[end + 2..];
        }
        output.push_str(rest);
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variable_crud_validates_names_preserves_ids_and_isolates_environments() {
        let mut a = EnvironmentVariables::new("http://localhost");
        let b = EnvironmentVariables::new("https://example.com");
        let id = a.save(None, " customer_id ", "customer_1", false).unwrap();
        assert_eq!(a.values().resolve("{{customer_id}}").unwrap(), "customer_1");
        assert!(b.values().resolve("{{customer_id}}").is_err());
        assert!(a.save(None, "customer_id", "duplicate", false).is_err());
        for name in ["", "bad name", "1bad", "{{bad}}", "坏名字"] {
            assert!(a.save(None, name, "", false).is_err());
        }
        assert_eq!(
            a.save(Some(id), "account_id", "account_2", true).unwrap(),
            id
        );
        assert!(a.values().resolve("{{customer_id}}").is_err());
        assert_eq!(a.values().resolve("{{account_id}}").unwrap(), "account_2");
        assert!(
            a.variables
                .iter()
                .find(|variable| variable.id == id)
                .unwrap()
                .secret
        );
        assert!(a.save(Some(id), "base_url", "bad", false).is_err());
        let removed = a.remove(id).unwrap();
        assert_eq!(removed.name, "account_id");
        assert!(a.values().resolve("{{account_id}}").is_err());
        assert!(a.remove(id).is_none());
        assert!(a.save(Some(id), "missing", "", false).is_err());
        let base = a
            .variables
            .iter()
            .find(|variable| variable.name == "base_url")
            .unwrap()
            .id;
        a.remove(base).unwrap();
        assert!(a.values().resolve("{{base_url}}").is_err());
    }
    #[test]
    fn table_save_is_atomic_supports_name_swaps_and_removes_deleted_rows() {
        let mut env = EnvironmentVariables::new("https://example.com");
        let before = env.variables.clone();
        assert!(
            env.replace(&[
                ("valid".into(), "one".into(), false),
                ("valid".into(), "two".into(), false)
            ])
            .is_err()
        );
        assert_eq!(env.variables, before);
        env.replace(&[
            ("workspace_id".into(), "https://example.com".into(), false),
            ("base_url".into(), "ws_personal".into(), true),
        ])
        .unwrap();
        assert_eq!(env.values().resolve("{{base_url}}").unwrap(), "ws_personal");
        assert!(
            env.variables
                .iter()
                .find(|v| v.name == "base_url")
                .unwrap()
                .secret
        );
        assert!(env.values().resolve("{{api_token}}").is_err());
        env.replace(&[]).unwrap();
        assert!(env.variables.is_empty());
    }
    #[test]
    fn nested_variables_detect_cycles_empty_values_and_missing_references() {
        let mut a = EnvironmentVariables::new("http://localhost");
        a.save(None, "host", "example.com", false).unwrap();
        let base = a.variables[0].id;
        a.save(Some(base), "base_url", "https://{{host}}", false)
            .unwrap();
        assert_eq!(
            a.values().resolve("{{base_url}}/{{host}}").unwrap(),
            "https://example.com/example.com"
        );
        let host = a
            .variables
            .iter()
            .find(|variable| variable.name == "host")
            .unwrap()
            .id;
        a.save(Some(host), "host", "{{base_url}}", false).unwrap();
        assert!(
            a.values()
                .resolve("{{base_url}}")
                .err()
                .unwrap()
                .contains("循环")
        );
        assert!(a.values().resolve("{{missing}}").is_err());
        assert!(a.values().resolve("{{host").is_err());
        assert!(a.values().resolve("{{api_token}}").is_err());
    }
}
