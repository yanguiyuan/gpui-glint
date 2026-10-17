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
/// Request-local action binding; flattened approval preserves older workspace data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeforeAction {
    #[serde(flatten)]
    pub approval: gpui_glint::plugins::Approval,
    #[serde(default)]
    pub action_id: String,
    #[serde(default = "action_enabled")]
    pub enabled: bool,
}
fn action_enabled() -> bool {
    true
}
impl std::ops::Deref for BeforeAction {
    type Target = gpui_glint::plugins::Approval;
    fn deref(&self) -> &Self::Target {
        &self.approval
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyFormat {
    #[default]
    Raw,
    UrlEncoded,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub plugins: Vec<BeforeAction>,
    #[serde(default)]
    pub extractors: Vec<Pair>,
    pub method: String,
    pub url: String,
    pub body: String,
    #[serde(default)]
    pub body_format: BodyFormat,
    #[serde(default)]
    pub form: Vec<Pair>,
    pub params: Vec<Pair>,
    pub headers: Vec<Pair>,
    pub cookies: Vec<Pair>,
    pub token: String,
    pub auth: bool,
    #[serde(default)]
    pub skip_tls_verification: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Exchange {
    #[serde(default)]
    pub curl: Option<String>,
    pub response: Option<super::http::HttpResponse>,
    pub error: Option<String>,
    pub preview: Option<String>,
    pub traces: Vec<super::TraceEntry>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    #[serde(default)]
    pub environment_name: Option<String>,
    pub draft: Draft,
    pub label: String,
    pub time: String,
    pub detail: String,
    pub elapsed_ms: Option<u128>,
    #[serde(default)]
    pub exchange: Option<Exchange>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub tree_order: u64,
    pub name: String,
    pub collection: String,
    #[serde(rename = "folder", alias = "group", default)]
    pub group: String,
    pub draft: Draft,
    pub versions: Vec<Record>,
    pub history: Vec<Record>,
}
/// Each section is either inherited or replaced in full, including explicit empty values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultSection {
    Endpoint,
    Auth,
    Params,
    Headers,
    Cookies,
    Body,
    Plugins,
    Extractors,
    Settings,
}
impl DefaultSection {
    pub const ALL: [Self; 9] = [
        Self::Endpoint,
        Self::Auth,
        Self::Params,
        Self::Headers,
        Self::Cookies,
        Self::Body,
        Self::Plugins,
        Self::Extractors,
        Self::Settings,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Endpoint => "请求地址",
            Self::Auth => "身份验证",
            Self::Params => "参数",
            Self::Headers => "请求头",
            Self::Cookies => "Cookie",
            Self::Body => "请求体",
            Self::Plugins => "前置动作",
            Self::Extractors => "后置动作",
            Self::Settings => "请求设置",
        }
    }
    pub fn apply(self, from: &Draft, to: &mut Draft) {
        match self {
            Self::Endpoint => {
                to.url = from.url.clone();
                to.method = from.method.clone();
            }
            Self::Auth => {
                to.auth = from.auth;
                to.token = from.token.clone();
            }
            Self::Params => to.params = from.params.clone(),
            Self::Headers => to.headers = from.headers.clone(),
            Self::Cookies => to.cookies = from.cookies.clone(),
            Self::Body => {
                to.body = from.body.clone();
                to.body_format = from.body_format;
                to.form = from.form.clone();
            }
            Self::Plugins => to.plugins = from.plugins.clone(),
            Self::Extractors => to.extractors = from.extractors.clone(),
            Self::Settings => to.skip_tls_verification = from.skip_tls_verification,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DirectoryDefaults {
    pub sections: std::collections::BTreeSet<DefaultSection>,
    pub draft: Draft,
}
impl DirectoryDefaults {
    pub fn apply(&self, draft: &mut Draft) {
        for section in &self.sections {
            section.apply(&self.draft, draft);
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RequestGroups {
    #[serde(default)]
    pub defaults: std::collections::BTreeMap<String, DirectoryDefaults>,
    #[serde(default)]
    pub folder_order: std::collections::BTreeMap<String, u64>,
    pub collection: String,
    #[serde(rename = "folders", alias = "names", default)]
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
            folder_order: Default::default(),
            defaults: Default::default(),
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
            folder_order: Default::default(),
            defaults: Default::default(),
            names: vec![],
        })
    }
    pub fn request_defaults(&self, path: &str) -> Draft {
        let mut ancestors = vec![path];
        let mut parent = path;
        while !parent.is_empty() {
            parent = Self::parent(parent);
            ancestors.push(parent);
        }
        let mut draft = Draft::default();
        for path in ancestors.into_iter().rev() {
            if let Some(defaults) = self.defaults.get(path) {
                defaults.apply(&mut draft);
            }
        }
        draft
    }
    pub fn parent(path: &str) -> &str {
        path.rsplit_once('/').map_or("", |(parent, _)| parent)
    }
    pub fn contains(&self, path: &str) -> bool {
        path.is_empty() || self.names.iter().any(|folder| folder == path)
    }
    #[cfg(test)]
    pub fn add(&mut self, name: &str) -> Result<String, String> {
        self.add_child("", name)
    }
    pub fn add_child(&mut self, parent: &str, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("请输入目录名称".into());
        }
        if name.contains('/') {
            return Err("目录名称不能包含 /".into());
        }
        if !self.contains(parent) {
            return Err("父目录不存在".into());
        }
        let path = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        if self.names.contains(&path) {
            return Err("同级目录名称已存在".into());
        }
        self.names.push(path.clone());
        Ok(path)
    }
    pub fn rename(
        &mut self,
        old: &str,
        name: &str,
        requests: &mut [Request],
    ) -> Result<(), String> {
        if old.is_empty() || !self.names.iter().any(|folder| folder == old) {
            return Err("目录不存在".into());
        }
        let name = name.trim();
        if name.is_empty() {
            return Err("请输入目录名称".into());
        }
        if name.contains('/') {
            return Err("目录名称不能包含 /".into());
        }
        let parent = Self::parent(old);
        let new = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        if new != old && self.names.contains(&new) {
            return Err("同级目录名称已存在".into());
        }
        let prefix = format!("{old}/");
        let replace = |path: &mut String| {
            if path == old || path.starts_with(&prefix) {
                *path = format!("{new}{}", &path[old.len()..]);
            }
        };
        for path in &mut self.names {
            replace(path);
        }
        self.folder_order = std::mem::take(&mut self.folder_order)
            .into_iter()
            .map(|(mut path, order)| {
                replace(&mut path);
                (path, order)
            })
            .collect();
        self.defaults = std::mem::take(&mut self.defaults)
            .into_iter()
            .map(|(mut path, defaults)| {
                replace(&mut path);
                (path, defaults)
            })
            .collect();
        for request in requests
            .iter_mut()
            .filter(|r| r.collection == self.collection)
        {
            replace(&mut request.group);
        }
        Ok(())
    }
    /// Remove a directory subtree while keeping its requests in the parent directory.
    pub fn remove(&mut self, path: &str, requests: &mut [Request]) {
        if path.is_empty() || !self.names.iter().any(|folder| folder == path) {
            return;
        }
        let prefix = format!("{path}/");
        self.names
            .retain(|folder| folder != path && !folder.starts_with(&prefix));
        self.folder_order
            .retain(|folder, _| folder != path && !folder.starts_with(&prefix));
        self.defaults
            .retain(|folder, _| folder != path && !folder.starts_with(&prefix));
        for request in requests
            .iter_mut()
            .filter(|r| r.collection == self.collection)
        {
            if request.group == path || request.group.starts_with(&prefix) {
                request.group = Self::parent(path).to_owned();
            }
        }
    }
}
/// Visible preorder rows of the request tree. Empty paths represent collection roots.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeEntry {
    Directory {
        collection: usize,
        path: String,
        depth: usize,
        expanded: bool,
    },
    Request {
        index: usize,
        depth: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeNode {
    Directory { collection: usize, path: String },
    Request(usize),
}
pub use gpui_glint::components::TreeDropPlacement as Placement;

pub fn siblings(
    collections: &[RequestGroups],
    requests: &[Request],
    collection: usize,
    parent: &str,
) -> Vec<TreeNode> {
    let c = &collections[collection];
    let mut nodes = c
        .names
        .iter()
        .filter(|p| RequestGroups::parent(p) == parent)
        .map(|p| TreeNode::Directory {
            collection,
            path: p.clone(),
        })
        .chain(
            requests
                .iter()
                .enumerate()
                .filter(|(_, r)| r.collection == c.collection && r.group == parent)
                .map(|(i, _)| TreeNode::Request(i)),
        )
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| match node {
        TreeNode::Directory { path, .. } => c.folder_order.get(path).copied().unwrap_or(0),
        TreeNode::Request(i) => requests[*i].tree_order,
    });
    nodes
}

/// Move metadata without reindexing requests, preserving tabs, responses and history.
pub fn move_node(
    collections: &mut [RequestGroups],
    requests: &mut [Request],
    source: &TreeNode,
    target: &TreeNode,
    placement: Placement,
) -> Result<TreeNode, String> {
    if source == target {
        return Ok(source.clone());
    }
    if let TreeNode::Directory { collection, path } = target {
        if !collections
            .get(*collection)
            .is_some_and(|c| c.contains(path))
        {
            return Err("目标目录不存在".into());
        }
    }
    if matches!(target, TreeNode::Request(_)) && placement == Placement::Inside {
        return Err("请求是叶子节点，不能包含其他节点".into());
    }
    let (destination, parent) = match target {
        TreeNode::Directory { collection, path } => (
            *collection,
            if placement == Placement::Inside {
                path.clone()
            } else {
                if path.is_empty() {
                    return Err("请将节点放入集合内".into());
                }
                RequestGroups::parent(path).to_owned()
            },
        ),
        TreeNode::Request(i) => {
            let request = requests.get(*i).ok_or("请求不存在")?;
            (
                collections
                    .iter()
                    .position(|c| c.collection == request.collection)
                    .ok_or("集合不存在")?,
                request.group.clone(),
            )
        }
    };
    let destination_collection = collections.get(destination).ok_or("集合不存在")?;
    if !destination_collection.contains(&parent) {
        return Err("目录不存在".into());
    }
    let destination_name = destination_collection.collection.clone();
    let moved = match source {
        TreeNode::Request(i) => {
            let request = requests.get_mut(*i).ok_or("请求不存在")?;
            request.collection = destination_name;
            request.group = parent.clone();
            source.clone()
        }
        TreeNode::Directory { collection, path } => {
            if path.is_empty() {
                return Err("集合只能在顶层排序".into());
            }
            let c = collections.get(*collection).ok_or("集合不存在")?;
            if !c.names.contains(path) {
                return Err("目录不存在".into());
            }
            if *collection == destination
                && (parent == *path || parent.starts_with(&format!("{path}/")))
            {
                return Err("不能将目录移入自身或其子目录".into());
            }
            let name = path.rsplit('/').next().unwrap();
            let next = if parent.is_empty() {
                name.into()
            } else {
                format!("{parent}/{name}")
            };
            if (*collection != destination || next != *path)
                && collections[destination].names.contains(&next)
            {
                return Err("目标目录下存在同名目录".into());
            }
            let old_name = c.collection.clone();
            let subtree = |p: &str| p == path || p.starts_with(&format!("{path}/"));
            let mapping = c
                .names
                .iter()
                .filter(|p| subtree(p))
                .map(|p| {
                    (
                        p.clone(),
                        format!("{next}{}", &p[path.len()..]),
                        c.folder_order.get(p).copied().unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>();
            let defaults = collections[*collection]
                .defaults
                .iter()
                .filter(|(p, _)| subtree(p))
                .map(|(p, d)| (format!("{next}{}", &p[path.len()..]), d.clone()))
                .collect::<Vec<_>>();
            collections[*collection].defaults.retain(|p, _| !subtree(p));
            collections[destination].defaults.extend(defaults);
            collections[*collection].names.retain(|p| !subtree(p));
            collections[*collection]
                .folder_order
                .retain(|p, _| !subtree(p));
            for (old, new, order) in &mapping {
                collections[destination].names.push(new.clone());
                collections[destination]
                    .folder_order
                    .insert(new.clone(), *order);
                for request in requests
                    .iter_mut()
                    .filter(|r| r.collection == old_name && r.group == *old)
                {
                    request.collection = destination_name.clone();
                    request.group = new.clone();
                }
            }
            TreeNode::Directory {
                collection: destination,
                path: next,
            }
        }
    };
    let mut nodes = siblings(collections, requests, destination, &parent);
    nodes.retain(|n| *n != moved);
    let position = if placement == Placement::Inside {
        nodes.len()
    } else {
        nodes
            .iter()
            .position(|n| n == target)
            .ok_or("目标节点不存在")?
            + usize::from(placement == Placement::After)
    };
    nodes.insert(position, moved.clone());
    for (i, node) in nodes.iter().enumerate() {
        let order = (i as u64 + 1) * 1024;
        match node {
            TreeNode::Request(index) => requests[*index].tree_order = order,
            TreeNode::Directory { path, .. } => {
                collections[destination]
                    .folder_order
                    .insert(path.clone(), order);
            }
        }
    }
    Ok(moved)
}

pub fn tree_list(
    collections: &[RequestGroups],
    requests: &[Request],
    collapsed: &std::collections::HashSet<(usize, String)>,
    query: &str,
) -> gpui_glint::components::TreeList {
    use gpui_glint::components::{TreeList, TreeListNode};
    fn directory(
        collections: &[RequestGroups],
        requests: &[Request],
        collection: usize,
        path: &str,
    ) -> TreeListNode {
        let c = &collections[collection];
        let label = if path.is_empty() {
            c.collection.clone()
        } else {
            path.rsplit('/').next().unwrap_or(path).into()
        };
        TreeListNode::branch(
            format!("request-directory:{collection}:{path}"),
            label,
            siblings(collections, requests, collection, path)
                .into_iter()
                .map(|node| match node {
                    TreeNode::Directory { path, .. } => {
                        directory(collections, requests, collection, &path)
                    }
                    TreeNode::Request(index) => {
                        let r = &requests[index];
                        TreeListNode::leaf(format!("request-leaf:{index}"), r.name.clone())
                            .search_text(format!(
                                "{} {} {} {}",
                                r.name, r.group, r.draft.method, r.draft.url
                            ))
                    }
                }),
        )
        .search_text(path.to_owned())
    }
    TreeList::new((0..collections.len()).map(|index| directory(collections, requests, index, "")))
        .collapsed(
            collapsed
                .iter()
                .map(|(collection, path)| format!("request-directory:{collection}:{path}").into()),
        )
        .query(query.to_owned())
}
pub fn tree_entry(row: &gpui_glint::components::TreeListRow) -> TreeEntry {
    if let Some(index) = row.id.strip_prefix("request-leaf:") {
        TreeEntry::Request {
            index: index.parse().unwrap(),
            depth: row.depth,
        }
    } else {
        let (collection, path) = row
            .id
            .strip_prefix("request-directory:")
            .unwrap()
            .split_once(':')
            .unwrap();
        TreeEntry::Directory {
            collection: collection.parse().unwrap(),
            path: path.into(),
            depth: row.depth,
            expanded: row.expanded,
        }
    }
}
#[cfg(test)]
pub fn tree_entries(
    collections: &[RequestGroups],
    requests: &[Request],
    collapsed: &std::collections::HashSet<(usize, String)>,
    query: &str,
) -> Vec<TreeEntry> {
    tree_list(collections, requests, collapsed, query)
        .visible_rows()
        .iter()
        .map(tree_entry)
        .collect()
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
                tree_order: 0,
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
            plugins: vec![],
            extractors: vec![],
            method: "GET".into(),
            url: String::new(),
            body: String::new(),
            body_format: BodyFormat::Raw,
            form: vec![],
            params: vec![],
            headers: vec![],
            cookies: vec![],
            token: String::new(),
            auth: false,
            skip_tls_verification: false,
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
    fn moves_preserve_request_indices_and_reorder_mixed_siblings() {
        let mut requests = test_requests();
        let mut c = RequestGroups::create_collection("Test API", &[]).unwrap();
        c.add_child("", "A").unwrap();
        c.add_child("A", "Child").unwrap();
        c.add_child("", "B").unwrap();
        requests[0].group = "A/Child".into();
        requests[1].group = "".into();
        let before = requests[0].draft.clone();
        let mut collections = vec![c, RequestGroups::create_collection("Other", &[]).unwrap()];
        let a = TreeNode::Directory {
            collection: 0,
            path: "A".into(),
        };
        let b = TreeNode::Directory {
            collection: 0,
            path: "B".into(),
        };
        let child = TreeNode::Directory {
            collection: 0,
            path: "A/Child".into(),
        };
        assert!(
            move_node(
                &mut collections,
                &mut requests,
                &a,
                &child,
                Placement::Inside
            )
            .is_err()
        );
        move_node(
            &mut collections,
            &mut requests,
            &TreeNode::Request(1),
            &a,
            Placement::Before,
        )
        .unwrap();
        assert_eq!(
            siblings(&collections, &requests, 0, "")[0],
            TreeNode::Request(1)
        );
        move_node(&mut collections, &mut requests, &a, &b, Placement::Inside).unwrap();
        assert_eq!(requests[0].group, "B/A/Child");
        assert_eq!(requests[0].draft, before);
        assert!(collections[0].contains("B/A/Child"));
        let moved = TreeNode::Directory {
            collection: 0,
            path: "B/A".into(),
        };
        let other = TreeNode::Directory {
            collection: 1,
            path: "".into(),
        };
        move_node(
            &mut collections,
            &mut requests,
            &moved,
            &other,
            Placement::Inside,
        )
        .unwrap();
        assert_eq!(requests[0].collection, "Other");
        assert_eq!(requests[0].group, "A/Child");
        assert!(!collections[0].contains("B/A"));
        assert!(collections[1].contains("A/Child"));
        collections[1].add_child("", "B").unwrap();
        let b_other = TreeNode::Directory {
            collection: 1,
            path: "B".into(),
        };
        assert!(
            move_node(
                &mut collections,
                &mut requests,
                &b,
                &other,
                Placement::Inside
            )
            .is_err()
        );
        assert!(collections[0].contains("B"));
        move_node(
            &mut collections,
            &mut requests,
            &TreeNode::Request(1),
            &b_other,
            Placement::Inside,
        )
        .unwrap();
        assert_eq!(requests[1].collection, "Other");
        assert_eq!(requests[1].group, "B");
    }
    #[test]
    fn nested_directories_rename_and_delete_preserve_requests() {
        let mut collection = RequestGroups::create_collection("Test API", &[]).unwrap();
        collection.add_child("", "用户").unwrap();
        collection.add_child("用户", "详情").unwrap();
        collection.add_child("用户/详情", "内部").unwrap();
        collection.add_child("", "用户备份").unwrap();
        assert!(collection.add_child("不存在", "子目录").is_err());
        assert!(collection.add_child("用户", "详情").is_err());
        assert!(collection.add_child("用户", "bad/name").is_err());
        collection.add_child("用户备份", "详情").unwrap();
        let mut requests = test_requests();
        requests[0].group.clear();
        requests[1].group = "用户/详情/内部".into();
        requests[2].group = "用户备份/详情".into();
        requests[3].collection = "Other API".into();
        requests[3].group = "用户/详情".into();
        let original_drafts: Vec<_> = requests.iter().map(|r| r.draft.clone()).collect();
        collection.rename("用户", "客户", &mut requests).unwrap();
        assert!(collection.contains("客户/详情/内部"));
        assert_eq!(requests[1].group, "客户/详情/内部");
        assert_eq!(requests[2].group, "用户备份/详情");
        assert_eq!(requests[3].group, "用户/详情");
        collection.remove("客户/详情", &mut requests);
        assert_eq!(requests[1].group, "客户");
        assert!(!collection.contains("客户/详情/内部"));
        collection.remove("客户", &mut requests);
        assert!(requests[0].group.is_empty() && requests[1].group.is_empty());
        assert_eq!(
            original_drafts,
            requests.iter().map(|r| r.draft.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn tree_supports_root_requests_deep_nesting_collapse_and_search() {
        let mut c = RequestGroups::create_collection("Test API", &[]).unwrap();
        let mut path = String::new();
        for _ in 0..24 {
            path = c.add_child(&path, "目录").unwrap();
        }
        let mut requests = test_requests()[..2].to_vec();
        requests[0].group.clear();
        requests[0].name = "根部请求".into();
        requests[1].group = path;
        requests[1].name = "深层请求".into();
        let collections = vec![c, RequestGroups::create_collection("Empty", &[]).unwrap()];
        let mut collapsed = std::collections::HashSet::new();
        let rows = tree_entries(&collections, &requests, &collapsed, "");
        assert!(rows.contains(&TreeEntry::Request { index: 0, depth: 1 }));
        assert!(rows.contains(&TreeEntry::Request {
            index: 1,
            depth: 25
        }));
        collapsed.insert((0, "目录".into()));
        assert!(
            !tree_entries(&collections, &requests, &collapsed, "")
                .iter()
                .any(|r| matches!(r, TreeEntry::Request { index: 1, .. }))
        );
        let searched = tree_entries(&collections, &requests, &collapsed, "深层请求");
        assert_eq!(searched.len(), 26);
        assert_eq!(
            searched.last(),
            Some(&TreeEntry::Request {
                index: 1,
                depth: 25
            })
        );
        assert!(!searched.contains(&TreeEntry::Request { index: 0, depth: 1 }));
        collapsed.insert((0, String::new()));
        assert!(
            tree_entries(&collections, &requests, &collapsed, "Test API")
                .contains(&TreeEntry::Request { index: 0, depth: 1 })
        );
        assert_eq!(
            tree_entries(&collections, &requests, &collapsed, "").len(),
            2
        );
        assert!(tree_entries(&collections, &requests, &collapsed, "not-found").is_empty());
    }

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
        assert_eq!(requests.last().unwrap().group, "");
        assert_eq!(requests[0].group, "资源");
        assert_eq!(requests[0].draft, original);
        assert_eq!(requests.last().unwrap().draft, original);
        assert!(collections[1].names.is_empty());
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
                .filter(|request| request.group.is_empty())
                .count(),
            3
        );
        assert!(!groups.names.iter().any(|name| name == "未分组"));
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
        assert!(!groups.names.iter().any(|name| name == "未分组"));
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
