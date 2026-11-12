//! Data-only TextMate language packs. No plugin scripts are evaluated.
use crate::components::TokenKind;
use irosashi::{Registry, RegistryBuilder, Session, TokenizeOptions};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
};

const GRAMMAR_LIMIT: usize = 512 * 1024;
const LANGUAGE_LIMIT: usize = 128;

#[derive(Deserialize)]
struct Manifest {
    id: String,
    api_version: u32,
    contributions: Vec<Contribution>,
}
#[derive(Deserialize)]
struct Contribution {
    kind: String,
    #[serde(default)]
    hook: String,
    data: Option<Language>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Language {
    language: String,
    grammar: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    filenames: Vec<String>,
}

/// Immutable snapshot. Install/remove packs and restart the host to refresh.
pub struct LanguageRegistry {
    engine: Arc<Registry>,
    aliases: BTreeMap<String, String>,
    extensions: BTreeMap<String, String>,
    filenames: BTreeMap<String, String>,
    scopes: BTreeSet<String>,
    /// Invalid packs are skipped independently; hosts may display these diagnostics.
    pub diagnostics: Vec<String>,
}

fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("file exceeds language-pack size limit".into());
    }
    Ok(bytes)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._+-".contains(&b))
}

impl LanguageRegistry {
    fn empty() -> Self {
        Self {
            engine: Arc::new(
                RegistryBuilder::without_bundled_assets()
                    .without_default_tables()
                    .build()
                    .expect("empty registry"),
            ),
            aliases: BTreeMap::new(),
            extensions: BTreeMap::new(),
            filenames: BTreeMap::new(),
            scopes: BTreeSet::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Load immediate child directories containing `glint-plugin.json` in name order.
    /// A missing directory is an empty registry. A bad pack never discards valid packs.
    pub fn load_dir(root: &Path) -> Self {
        let mut result = Self::empty();
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    result
                        .diagnostics
                        .push(format!("{}: {error}", root.display()));
                }
                return result;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) if entry.path().is_dir() => paths.push(entry.path()),
                Ok(_) => {}
                Err(error) => result.diagnostics.push(error.to_string()),
            }
        }
        paths.sort();
        for path in paths.into_iter().take(LANGUAGE_LIMIT) {
            if let Err(error) = result.load_pack(&path) {
                result
                    .diagnostics
                    .push(format!("{}: {error}", path.display()));
            }
        }
        result
    }

    fn load_pack(&mut self, directory: &Path) -> Result<(), String> {
        let manifest: Manifest = serde_json::from_slice(&read_limited(
            &directory.join("glint-plugin.json"),
            64 * 1024,
        )?)
        .map_err(|e| e.to_string())?;
        if manifest.api_version != 1
            || !identifier(&manifest.id)
            || manifest.contributions.is_empty()
            || manifest.contributions.len() > 32
        {
            return Err("invalid language-pack manifest".into());
        }
        // Stage every contribution so a failed package cannot leave partial registrations.
        let mut engine = self.engine.clone();
        let mut aliases = self.aliases.clone();
        let mut extensions = self.extensions.clone();
        let mut filenames = self.filenames.clone();
        let mut scopes = self.scopes.clone();
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        for contribution in manifest.contributions {
            if contribution.kind != "syntax.language" || !contribution.hook.is_empty() {
                return Err(
                    "language packs accept only declarative syntax.language contributions".into(),
                );
            }
            let language = contribution.data.ok_or("missing language declaration")?;
            if !identifier(&language.language)
                || aliases.contains_key(&language.language)
                || scopes.len() >= LANGUAGE_LIMIT
            {
                return Err("invalid or duplicate language id".into());
            }
            let relative = Path::new(&language.grammar);
            if relative.as_os_str().is_empty()
                || !relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
            {
                return Err("grammar path must stay inside its pack".into());
            }
            let path = directory
                .join(relative)
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if !path.starts_with(&directory) {
                return Err("grammar path escapes its pack".into());
            }
            let bytes = read_limited(&path, GRAMMAR_LIMIT)?;
            let json: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let scope = json
                .get("scopeName")
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .ok_or("grammar requires scopeName")?;
            if !scopes.insert(scope.into()) {
                return Err("duplicate grammar scopeName".into());
            }
            let next = engine
                .with_grammar(&language.language, bytes)
                .map_err(|e| e.to_string())?;
            // Force structural validation before publishing the snapshot.
            next.grammar(&language.language)
                .map_err(|e| e.to_string())?;
            engine = Arc::new(next);
            for alias in std::iter::once(language.language.clone())
                .chain(language.aliases)
                .chain(language.extensions.iter().cloned())
            {
                let alias = alias.to_ascii_lowercase();
                if !identifier(&alias) {
                    return Err("invalid language alias/extension".into());
                }
                if aliases
                    .get(&alias)
                    .is_some_and(|id| id != &language.language)
                {
                    return Err(format!("duplicate language alias: {alias}"));
                }
                aliases.insert(alias, language.language.clone());
            }
            for extension in language.extensions {
                extensions.insert(extension.to_ascii_lowercase(), language.language.clone());
            }
            for filename in language.filenames {
                if filename.is_empty()
                    || filename.contains(['/', '\\'])
                    || filename.len() > 256
                    || filenames.contains_key(&filename)
                {
                    return Err("invalid or duplicate filename association".into());
                }
                filenames.insert(filename, language.language.clone());
            }
        }
        self.engine = engine;
        self.aliases = aliases;
        self.extensions = extensions;
        self.filenames = filenames;
        self.scopes = scopes;
        Ok(())
    }

    /// Explicit file-name associations win over extension associations.
    pub fn language_for_path(&self, path: &str) -> Option<String> {
        let filename = path.rsplit(['/', '\\']).next()?;
        self.filenames
            .get(filename)
            .or_else(|| {
                let (_, extension) = filename.rsplit_once('.')?;
                self.extensions.get(&extension.to_ascii_lowercase())
            })
            .cloned()
    }

    pub(crate) fn tokens(
        &self,
        source: &str,
        language: &str,
    ) -> Option<Vec<(Range<usize>, TokenKind)>> {
        let id = self.aliases.get(&language.to_ascii_lowercase())?;
        // Preserve a full plain run when an installed grammar cannot handle the buffer.
        let plain = || vec![(0..source.len(), TokenKind::Plain)];
        if source.len() > 2 * 1024 * 1024 {
            return Some(plain());
        }
        let grammar = self.engine.grammar(id).ok()?;
        let mut session = Session::new(grammar, self.engine.clone());
        let result = session.tokenize(
            source,
            TokenizeOptions {
                max_line_length: Some(16 * 1024),
            },
        );
        let mut spans = Vec::new();
        let mut offset = 0;
        for (line, tokens) in source.split_inclusive('\n').zip(result.lines) {
            let mut cursor = 0;
            for token in tokens {
                if token.start < cursor
                    || token.end < token.start
                    || token.end > line.len()
                    || !line.is_char_boundary(token.start)
                    || !line.is_char_boundary(token.end)
                {
                    return Some(plain());
                }
                if cursor < token.start {
                    spans.push((offset + cursor..offset + token.start, TokenKind::Plain));
                }
                if token.start < token.end {
                    let kind = session
                        .scope_names(token.scopes)
                        .into_iter()
                        .rev()
                        .find_map(scope_kind)
                        .unwrap_or(TokenKind::Plain);
                    spans.push((offset + token.start..offset + token.end, kind));
                }
                cursor = token.end;
            }
            if cursor < line.len() {
                spans.push((offset + cursor..offset + line.len(), TokenKind::Plain));
            }
            offset += line.len();
        }
        if offset < source.len() {
            spans.push((offset..source.len(), TokenKind::Plain));
        }
        Some(spans)
    }
}

fn scope_kind(scope: &str) -> Option<TokenKind> {
    Some(match scope.split('.').next()? {
        "comment" => TokenKind::Comment,
        "string" => TokenKind::String,
        "keyword" | "storage" => TokenKind::Key,
        "constant" if scope.starts_with("constant.numeric") => TokenKind::Number,
        "constant" => TokenKind::Key,
        "support" | "entity" => TokenKind::Type,
        _ => return None,
    })
}

/// Override with `GLINT_SYNTAX_DIR`; otherwise use `<agent data dir>/syntax`.
pub fn default_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("GLINT_SYNTAX_DIR") {
        return path.into();
    }
    if let Some(path) = std::env::var_os("GLINT_AGENT_DATA_DIR") {
        return PathBuf::from(path).join("syntax");
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("Glint Agent/syntax")
}

/// Process-wide startup snapshot. No file access occurs during subsequent highlighting.
pub fn global() -> &'static LanguageRegistry {
    static REGISTRY: OnceLock<LanguageRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry = LanguageRegistry::load_dir(&default_directory());
        for diagnostic in &registry.diagnostics {
            eprintln!("Language pack: {diagnostic}");
        }
        registry
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(root: &Path, name: &str) -> PathBuf {
        let target = root.join(name);
        std::fs::create_dir_all(&target).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/language-packs/json");
        for file in ["glint-plugin.json", "json.tmLanguage.json"] {
            std::fs::copy(source.join(file), target.join(file)).unwrap();
        }
        target
    }

    fn edit_manifest(pack: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
        let path = pack.join("glint-plugin.json");
        let mut value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        edit(&mut value);
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[test]
    fn external_grammar_preserves_unicode_crlf_and_scope_mapping() {
        let root = tempfile::tempdir().unwrap();
        install(root.path(), "json");
        let registry = LanguageRegistry::load_dir(root.path());
        assert!(
            registry.diagnostics.is_empty(),
            "{:?}",
            registry.diagnostics
        );
        assert_eq!(
            registry.language_for_path(r"D:\项目\case.JSON").as_deref(),
            Some("json")
        );
        let source = "{\r\n  \"中文😀\": \"值\\n\",\r\n  \"count\": 42,\r\n  \"ok\": true,\r\n  \"none\": null\r\n}";
        let tokens = registry.tokens(source, "JSON").unwrap();
        let mut cursor = 0;
        for (range, _) in &tokens {
            assert_eq!(range.start, cursor);
            assert!(source.is_char_boundary(range.start) && source.is_char_boundary(range.end));
            cursor = range.end;
        }
        assert_eq!(cursor, source.len());
        for (text, expected) in [
            ("中文😀", TokenKind::Type),
            ("值\\n", TokenKind::String),
            ("42", TokenKind::Number),
            ("true", TokenKind::Key),
            ("null", TokenKind::Key),
        ] {
            let at = source.find(text).unwrap();
            assert!(
                tokens
                    .iter()
                    .any(|(range, kind)| range.contains(&at) && *kind == expected),
                "{text}: {tokens:?}"
            );
        }
        assert_eq!(tokens, registry.tokens(source, "json").unwrap());
        assert!(registry.tokens(source, "unknown").is_none());
    }

    #[test]
    fn bad_package_is_atomic_and_does_not_disable_other_packages() {
        let root = tempfile::tempdir().unwrap();
        let bad = install(root.path(), "a-bad");
        std::fs::write(bad.join("json.tmLanguage.json"), b"{broken").unwrap();
        install(root.path(), "b-valid");
        let registry = LanguageRegistry::load_dir(root.path());
        assert_eq!(registry.diagnostics.len(), 1);
        assert_eq!(registry.scopes.len(), 1);
        assert!(registry.tokens("{\"a\": 1}", "json").is_some());
    }

    #[test]
    fn rejects_traversal_runtime_hooks_duplicate_languages_and_oversized_grammars() {
        for variant in 0..4 {
            let root = tempfile::tempdir().unwrap();
            let pack = install(root.path(), "bad");
            match variant {
                0 => edit_manifest(&pack, |v| {
                    v["contributions"][0]["data"]["grammar"] = "../outside.json".into()
                }),
                1 => edit_manifest(&pack, |v| v["contributions"][0]["hook"] = "run.code".into()),
                2 => edit_manifest(&pack, |v| {
                    let duplicate = v["contributions"][0].clone();
                    v["contributions"].as_array_mut().unwrap().push(duplicate);
                }),
                _ => std::fs::write(
                    pack.join("json.tmLanguage.json"),
                    vec![b' '; GRAMMAR_LIMIT + 1],
                )
                .unwrap(),
            }
            let registry = LanguageRegistry::load_dir(root.path());
            assert_eq!(registry.diagnostics.len(), 1, "variant {variant}");
            assert!(registry.aliases.is_empty());
        }
    }

    #[test]
    fn filename_associations_and_long_lines_keep_complete_source_ranges() {
        let root = tempfile::tempdir().unwrap();
        let pack = install(root.path(), "json");
        edit_manifest(&pack, |v| {
            v["contributions"][0]["data"]["filenames"] = serde_json::json!(["glint.lock"])
        });
        let registry = LanguageRegistry::load_dir(root.path());
        assert_eq!(
            registry.language_for_path("/test/glint.lock").as_deref(),
            Some("json")
        );
        assert_eq!(registry.language_for_path("/test/glint.txt"), None);
        for source in [
            "".to_owned(),
            "\r\n".into(),
            "中".repeat(17000),
            "{\"a\": 1}\n".into(),
        ] {
            let tokens = registry.tokens(&source, "json").unwrap();
            assert_eq!(
                tokens
                    .iter()
                    .map(|(r, _)| &source[r.clone()])
                    .collect::<String>(),
                source
            );
        }
    }

    #[test]
    fn duplicate_pack_does_not_override_an_installed_language() {
        let root = tempfile::tempdir().unwrap();
        install(root.path(), "a");
        install(root.path(), "b");
        let registry = LanguageRegistry::load_dir(root.path());
        assert_eq!(registry.diagnostics.len(), 1);
        assert_eq!(registry.scopes.len(), 1);
    }
}
