//! Structured file context stays separate from the compact token shown in the prompt.
use super::Icon;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposerReference {
    pub token: String,
    pub path: String,
    pub directory: bool,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub excerpt: Option<String>,
    /// Display-only bounded snapshot; never included in a long reference's model input.
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub preview_truncated: bool,
    /// Clipboard content is carried in full, rather than read from a project path.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub text_attachment: bool,
    /// Merge-conflict snapshots get a dedicated diff icon and attachment wording.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diff_attachment: bool,
}
impl ComposerReference {
    pub fn file(path: impl Into<String>, directory: bool) -> Self {
        Self {
            token: String::new(),
            path: path.into(),
            directory,
            start_line: None,
            end_line: None,
            excerpt: None,
            preview: None,
            preview_truncated: false,
            text_attachment: false,
            diff_attachment: false,
        }
    }
    pub fn pasted_text(name: impl Into<String>, text: &str) -> Self {
        let mut reference = Self::file(name, false);
        reference.text_attachment = true;
        reference.excerpt = Some(text.to_owned());
        reference.set_preview(text);
        reference
    }
    /// A merge-conflict file snapshot: shown as its own highlighted diff
    /// reference with a dedicated icon instead of inline untrusted content.
    pub fn conflict_snapshot(name: impl Into<String>, text: &str) -> Self {
        let mut reference = Self::pasted_text(name, text);
        reference.diff_attachment = true;
        reference
    }
    pub fn selection(path: impl Into<String>, start: usize, end: usize, text: &str) -> Self {
        let mut reference = Self::file(path, false);
        reference.set_preview(text);
        reference.start_line = Some(start);
        reference.end_line = Some(end);
        if text.len() <= 2048 && end.saturating_sub(start) < 20 {
            reference.excerpt = Some(text.into());
        }
        reference
    }
    pub fn set_preview(&mut self, text: &str) {
        let mut lines = 0;
        let end = text
            .char_indices()
            .find_map(|(index, c)| {
                if index + c.len_utf8() > 4096 || lines >= 24 {
                    Some(index)
                } else {
                    if c == '\n' {
                        lines += 1;
                    }
                    None
                }
            })
            .unwrap_or(text.len());
        self.preview = Some(text[..end].to_owned());
        self.preview_truncated = end < text.len();
    }
    pub fn icon(&self) -> Icon {
        if self.directory {
            Icon::Folder
        } else if self.diff_attachment {
            Icon::GitConflict
        } else if self.excerpt.is_some() {
            Icon::Code
        } else if self.start_line.is_some() {
            Icon::Link
        } else {
            Icon::Text
        }
    }
    pub fn label(&self) -> String {
        let display = crate::paths::display_path(&self.path);
        let name = display
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(&display);
        match (self.start_line, self.end_line) {
            (Some(start), Some(end)) => format!("@{name}:{start}-{end}"),
            _ => format!("@{name}"),
        }
    }
    pub fn is_present(&self, text: &str) -> bool {
        if self.token.is_empty() {
            return false;
        }
        text.match_indices(&self.token).any(|(start, _)| {
            (start == 0
                || text[..start]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace))
                && text[start + self.token.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_whitespace() || ",，。;；)]}".contains(c))
        })
    }
    pub fn prompt(&self) -> String {
        let range = match (self.start_line, self.end_line) {
            (Some(start), Some(end)) => format!("，行范围 {start}-{end}"),
            _ => String::new(),
        };
        if let Some(text) = &self.excerpt {
            let fence = "`".repeat(
                text.split(|c| c != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0)
                    .max(2)
                    + 1,
            );
            let kind = if self.diff_attachment {
                "冲突文件快照"
            } else if self.text_attachment {
                "粘贴文本附件"
            } else {
                "文件引用"
            };
            format!("{kind}：{}{}\n{fence}\n{text}\n{fence}", self.path, range)
        } else {
            format!(
                "{}引用：{}{}；请使用文件工具按需读取，不要假定已获得其内容。",
                if self.directory { "目录" } else { "文件" },
                self.path,
                range
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_directory_labels_do_not_expose_prefixes_or_change_source_paths() {
        let path = r"\\?\D:\项目\src\";
        let reference = ComposerReference::file(path, true);
        assert_eq!(reference.label(), "@src");
        assert_eq!(reference.path, path);
        assert!(reference.prompt().contains(path));
        assert_eq!(ComposerReference::file(r"\\?\D:\", true).label(), "@D:");
    }
    #[test]
    fn short_and_long_context_have_bounded_previews_and_distinct_model_payloads() {
        let short = ComposerReference::selection("/project/源.rs", 3, 4, "你好\nworld");
        assert_eq!(short.excerpt.as_deref(), Some("你好\nworld"));
        assert!(short.prompt().contains("你好\nworld"));
        assert_eq!(short.icon(), Icon::Code);
        let long = ComposerReference::selection("/project/源.rs", 3, 600, &"你好\n".repeat(598));
        assert!(long.excerpt.is_none());
        assert!(long.preview.as_ref().unwrap().len() <= 4096);
        assert!(long.preview_truncated);
        assert!(!long.prompt().contains("你好"));
        assert!(long.prompt().contains("3-600"));
        assert_eq!(long.icon(), Icon::Link);
        let wide = ComposerReference::selection("/project/源.rs", 1, 1, &"汉".repeat(10000));
        assert!(
            wide.preview
                .as_ref()
                .unwrap()
                .is_char_boundary(wide.preview.as_ref().unwrap().len())
        );
        assert!(wide.preview_truncated);
        assert_eq!(
            ComposerReference::file("/project/src", true).icon(),
            Icon::Folder
        );
    }
    #[test]
    fn tokens_match_whole_references_and_old_drafts_remain_readable() {
        let mut file = ComposerReference::file("/project/foo", false);
        assert!(!file.is_present("some text"));
        file.token = "@foo".into();
        assert!(file.is_present("解释 @foo，继续"));
        assert!(!file.is_present("@foo.rs"));
        assert!(!file.is_present("abc@foo"));
        let mut value = serde_json::to_value(&file).unwrap();
        value.as_object_mut().unwrap().remove("preview");
        value.as_object_mut().unwrap().remove("preview_truncated");
        assert_eq!(
            serde_json::from_value::<ComposerReference>(value).unwrap(),
            file
        );
    }
    #[test]
    fn conflict_snapshots_use_a_dedicated_icon_and_full_model_payload() {
        let content = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> main\n".repeat(10);
        let mut reference =
            ComposerReference::conflict_snapshot("冲突快照-handler.go.txt", &content);
        assert!(reference.text_attachment && reference.diff_attachment);
        assert_eq!(reference.icon(), Icon::GitConflict);
        assert!(
            reference
                .prompt()
                .starts_with("冲突文件快照：冲突快照-handler.go.txt\n")
        );
        assert!(reference.prompt().contains("<<<<<<< HEAD"));
        // The fenced payload grows when the snapshot itself contains backticks.
        assert!(reference.prompt().contains("```"));
        reference.token = reference.label();
        assert_eq!(reference.token, "@冲突快照-handler.go.txt");
        assert!(reference.is_present(&format!("请解决：\n{}", reference.token)));
        // Drafts saved before the dedicated diff flag existed stay readable and
        // fall back to the plain pasted-text treatment.
        let mut value = serde_json::to_value(&reference).unwrap();
        value.as_object_mut().unwrap().remove("diff_attachment");
        let old = serde_json::from_value::<ComposerReference>(value).unwrap();
        assert!(!old.diff_attachment && old.text_attachment);
        assert_eq!(old.excerpt, reference.excerpt);
        assert_eq!(old.icon(), Icon::Code);
    }
}
