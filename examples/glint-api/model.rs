use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub enabled: bool,
    pub key: String,
    pub value: String,
    pub note: String,
}
impl Pair {
    pub fn new(key: &str, value: &str, note: &str) -> Self {
        Self {
            enabled: true,
            key: key.into(),
            value: value.into(),
            note: note.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub method: String,
    pub url: String,
    pub body: String,
    pub params: Vec<Pair>,
    pub headers: Vec<Pair>,
    pub cookies: Vec<Pair>,
    pub token: String,
    pub auth: bool,
    pub environment: usize,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub draft: Draft,
    pub label: String,
    pub time: String,
    pub detail: String,
    pub elapsed_ms: Option<u128>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    pub name: String,
    pub collection: String,
    pub group: String,
    pub draft: Draft,
    pub versions: Vec<Record>,
    pub history: Vec<Record>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RequestGroups {
    pub collection: String,
    pub names: Vec<String>,
}
impl RequestGroups {
    #[cfg(test)]
    pub fn new(requests: &[Request]) -> Self {
        let mut names = Vec::new();
        for request in requests
            .iter()
            .filter(|request| request.collection == "Test API")
        {
            if !names.contains(&request.group) {
                names.push(request.group.clone());
            }
        }
        Self {
            collection: "Test API".into(),
            names,
        }
    }
    pub fn create_collection(name: &str, existing: &[Self]) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("请输入集合名称".into());
        }
        if existing
            .iter()
            .any(|collection| collection.collection == name)
        {
            return Err("集合名称已存在".into());
        }
        Ok(Self {
            collection: name.into(),
            names: vec![],
        })
    }
    pub fn add(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("请输入分组名称".into());
        }
        if self.names.iter().any(|existing| existing == name) {
            return Err("分组名称已存在".into());
        }
        self.names.push(name.to_owned());
        Ok(name.to_owned())
    }
    pub fn rename(
        &mut self,
        old: &str,
        name: &str,
        requests: &mut [Request],
    ) -> Result<(), String> {
        let name = name.trim();
        if old == "未分组" {
            return Err("未分组为系统分组".into());
        }
        if name.is_empty() {
            return Err("请输入分组名称".into());
        }
        let index = self
            .names
            .iter()
            .position(|group| group == old)
            .ok_or("分组不存在")?;
        if name != old && self.names.iter().any(|existing| existing == name) {
            return Err("分组名称已存在".into());
        }
        self.names[index] = name.to_owned();
        for request in requests
            .iter_mut()
            .filter(|request| request.collection == self.collection && request.group == old)
        {
            request.group = name.to_owned();
        }
        Ok(())
    }
    pub fn remove(&mut self, name: &str, requests: &mut [Request]) {
        if name == "未分组" {
            return;
        }
        self.names.retain(|group| group != name);
        let mut moved = false;
        for request in requests
            .iter_mut()
            .filter(|request| request.collection == self.collection && request.group == name)
        {
            request.group = "未分组".into();
            moved = true;
        }
        if moved && !self.names.iter().any(|group| group == "未分组") {
            self.names.push("未分组".into());
        }
    }
}
#[cfg(test)]
pub fn test_requests() -> Vec<Request> {
    ["资源", "资源", "资源", "集合"]
        .into_iter()
        .enumerate()
        .map(|(i, group)| {
            let draft = Draft {
                url: "{{base_url}}/v1/resources".into(),
                headers: vec![Pair::new("X-Workspace", "{{workspace_id}}", "")],
                auth: true,
                token: "{{api_token}}".into(),
                ..Draft::default()
            };
            Request {
                name: format!("Test request {i}"),
                collection: "Test API".into(),
                group: group.into(),
                draft,
                versions: vec![],
                history: vec![],
            }
        })
        .collect()
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            method: "GET".into(),
            url: String::new(),
            body: String::new(),
            params: vec![],
            headers: vec![],
            cookies: vec![],
            token: String::new(),
            auth: false,
            environment: 0,
        }
    }
}
impl Draft {
    /// Assemble enabled headers; Cookie overrides replace matching cookie names.
    pub fn resolved_headers(&self) -> Vec<Pair> {
        let mut headers = Vec::new();
        let mut cookies: Vec<(String, String)> = Vec::new();
        let mut set_cookie = |key: &str, value: &str| {
            let key = key.trim();
            if key.is_empty() {
                return;
            }
            if let Some(pair) = cookies.iter_mut().find(|(name, _)| name == key) {
                pair.1 = value.trim().into();
            } else {
                cookies.push((key.into(), value.trim().into()));
            }
        };
        for header in self
            .headers
            .iter()
            .filter(|p| p.enabled && !p.key.trim().is_empty())
        {
            if header.key.trim().eq_ignore_ascii_case("cookie") {
                for part in header.value.split(';') {
                    if let Some((key, value)) = part.split_once('=') {
                        set_cookie(key, value);
                    }
                }
            } else {
                headers.push(header.clone());
            }
        }
        for cookie in self.cookies.iter().filter(|p| p.enabled) {
            set_cookie(&cookie.key, &cookie.value);
        }
        if !cookies.is_empty() {
            headers.push(Pair::new(
                "Cookie",
                &cookies
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join("; "),
                "合并后的 Cookie",
            ));
        }
        if self.auth {
            headers.retain(|p| !p.key.trim().eq_ignore_ascii_case("authorization"));
            headers.push(Pair::new(
                "Authorization",
                &format!("Bearer {}", self.token.trim()),
                "身份验证生成",
            ));
        }
        headers
    }

    pub fn resolved_url(&self) -> Result<String, String> {
        let mut url = self.url.trim().to_owned();
        if !(url.starts_with("https://") || url.starts_with("http://"))
            || url.contains("{{")
            || url.contains(char::is_whitespace)
        {
            return Err("请输入有效的 HTTP URL，或使用 {{base_url}} 环境变量。".into());
        }
        let query: Vec<_> = self
            .params
            .iter()
            .filter(|p| p.enabled && !p.key.trim().is_empty())
            .map(|p| format!("{}={}", encode(&p.key), encode(&p.value)))
            .collect();
        if !query.is_empty() {
            let fragment = url.find('#').map(|i| url.split_off(i)).unwrap_or_default();
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&query.join("&"));
            url.push_str(&fragment);
        }
        Ok(url)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookie_overrides_merge_enabled_headers_and_preserve_values() {
        let mut draft = test_requests().remove(0).draft;
        draft.auth = false;
        draft.headers = vec![
            Pair::new("cOoKiE", "session=old; theme=light; token=a=b", ""),
            Pair::new("Accept", "application/json", ""),
        ];
        let mut disabled = Pair::new("theme", "dark", "");
        disabled.enabled = false;
        draft.cookies = vec![
            Pair::new("session", "new", ""),
            disabled,
            Pair::new("extra", "", ""),
            Pair::new("", "ignored", ""),
            Pair::new("session", "latest", ""),
        ];
        let headers = draft.resolved_headers();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].key, "Accept");
        assert_eq!(
            headers[1].value,
            "session=latest; theme=light; token=a=b; extra="
        );
        draft.headers[0].enabled = false;
        assert_eq!(draft.resolved_headers()[1].value, "session=latest; extra=");
        draft.cookies.clear();
        assert_eq!(draft.resolved_headers().len(), 1);
    }

    #[test]
    fn collections_validate_names_and_isolate_group_changes() {
        let mut requests = test_requests();
        let mut collections = vec![RequestGroups::new(&requests)];
        assert!(RequestGroups::create_collection("  ", &collections).is_err());
        assert!(RequestGroups::create_collection(" Test API ", &collections).is_err());
        let mut collection =
            RequestGroups::create_collection("  Other API  ", &collections).unwrap();
        assert_eq!(collection.collection, "Other API");
        assert!(collection.names.is_empty());
        collection.add("资源").unwrap();
        let mut other = self::test_requests().remove(0);
        other.collection = collection.collection.clone();
        requests.push(other);
        let original = requests[0].draft.clone();
        collections.push(collection);
        assert!(RequestGroups::create_collection("Other API", &collections).is_err());
        collections[1]
            .rename("资源", "资源管理", &mut requests)
            .unwrap();
        assert_eq!(requests[0].group, "资源");
        assert_eq!(requests.last().unwrap().group, "资源管理");
        collections[1].remove("资源管理", &mut requests);
        assert_eq!(requests.last().unwrap().group, "未分组");
        assert_eq!(requests[0].group, "资源");
        assert_eq!(requests[0].draft, original);
        assert_eq!(requests.last().unwrap().draft, original);
        assert!(collections[1].names.iter().any(|name| name == "未分组"));
        assert!(!collections[0].names.iter().any(|name| name == "未分组"));
    }

    #[test]
    fn group_crud_preserves_request_data_and_rejects_duplicate_names() {
        let mut requests = test_requests();
        let drafts: Vec<_> = requests
            .iter()
            .map(|request| request.draft.clone())
            .collect();
        let mut groups = RequestGroups::new(&requests);
        assert_eq!(groups.add("  空分组  ").unwrap(), "空分组");
        assert!(groups.add("空分组").is_err());
        assert!(groups.add("  ").is_err());
        assert!(groups.rename("资源", "集合", &mut requests).is_err());
        groups.rename("资源", "资源管理", &mut requests).unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.group == "资源管理")
                .count(),
            3
        );
        groups.remove("资源管理", &mut requests);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.group == "未分组")
                .count(),
            3
        );
        assert!(groups.names.iter().any(|name| name == "未分组"));
        assert_eq!(
            drafts,
            requests
                .iter()
                .map(|request| request.draft.clone())
                .collect::<Vec<_>>()
        );
        groups.remove("空分组", &mut requests);
        assert!(!groups.names.iter().any(|name| name == "空分组"));
        groups.remove("未分组", &mut requests);
        assert!(groups.names.iter().any(|name| name == "未分组"));
    }

    #[test]
    fn query_resolves_environment_and_encodes_enabled_pairs() {
        let mut d = test_requests().remove(0).draft;
        d.url = "http://localhost:8080/v1/resources".into();
        d.url.push_str("?sort=new#result");
        d.params = vec![
            Pair::new("q", "雾 夜&", ""),
            Pair {
                enabled: false,
                ..Pair::new("secret", "ignored", "")
            },
        ];
        assert_eq!(
            d.resolved_url().unwrap(),
            "http://localhost:8080/v1/resources?sort=new&q=%E9%9B%BE%20%E5%A4%9C%26#result"
        );
    }
}
