//! AI-written commit messages from the index (or working tree) diff.
use super::*;
use gpui_glint::components::{Icon, PromptEditor};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Built-in system prompt. The settings page can replace it per user.
pub const DEFAULT_PROMPT: &str = "你是 Git 提交信息生成器。根据改动摘要，只输出一行提交信息，格式为 type(module):中文摘要，例如 feat(editor):支持粘贴图片 或 bugfix(git):修复暂存区刷新。新增功能用 feat，修复缺陷用 bugfix，其他改动按主要目的选用 refactor、perf、docs、test、build、ci 或 chore。module 使用实际模块名。使用半角括号和冒号，冒号后直接写摘要。整行不超过 50 个字，末尾不加句号。不要虚构改动内容。";

/// Persisted preferences for the commit-message wand.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// `None` follows the active chat provider and model.
    #[serde(default)]
    pub provider_id: Option<Uuid>,
    #[serde(default)]
    pub model: String,
    /// Empty falls back to [`DEFAULT_PROMPT`].
    #[serde(default)]
    pub prompt: String,
}

impl Settings {
    pub fn prompt(&self) -> &str {
        let custom = self.prompt.trim();
        if custom.is_empty() {
            DEFAULT_PROMPT
        } else {
            custom
        }
    }

    pub fn custom_prompt(&self) -> bool {
        !self.prompt.trim().is_empty()
    }
}

/// Strips fences, labels and wrapping quotes the model may add around the
/// message it was asked to produce.
pub fn tidy(raw: &str) -> String {
    let mut lines: Vec<&str> = raw
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .map(|line| line.trim_end())
        .collect();
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let text = lines.join("\n");
    let text = text.trim();
    let text = [
        "提交信息：",
        "提交信息:",
        "Commit message:",
        "commit message:",
    ]
    .iter()
    .find_map(|prefix| text.strip_prefix(prefix))
    .unwrap_or(text)
    .trim();
    let text = text.trim_matches(|c| matches!(c, '"' | '\u{201c}' | '\u{201d}' | '`'));
    let text = text.trim();
    if text.chars().count() > 2000 {
        text.chars().take(2000).collect()
    } else {
        text.to_owned()
    }
}

impl AgentWorkspace {
    /// The provider writing commit messages: the footer's own choice, or the
    /// active chat model.
    pub(super) fn commit_model_config(&self) -> Result<provider::Config, String> {
        let settings = &self.history.commit_message;
        let Some(id) = settings.provider_id else {
            let config = self.provider.clone();
            if !config.enabled {
                return Err("请先连接模型供应商，再生成提交信息".into());
            }
            config.validate()?;
            return Ok(config);
        };
        let mut config = self
            .catalog
            .providers
            .iter()
            .find(|provider| provider.id == id && provider.enabled)
            .cloned()
            .ok_or("提交信息使用的供应商已不可用，请重新选择模型")?;
        if !settings.model.is_empty() {
            config.model = settings.model.clone();
        }
        config.validate()?;
        Ok(config)
    }

    /// `(value, label)` rows for the footer's model menu; `inherit` follows
    /// the chat model.
    pub(super) fn commit_model_options(&self) -> Vec<(String, String)> {
        let mut options = vec![("inherit".to_owned(), "跟随当前模型".to_owned())];
        for profile in self
            .catalog
            .providers
            .iter()
            .filter(|profile| profile.enabled && profile.validate().is_ok())
        {
            let models = if profile.models.is_empty() && profile.kind == provider::Kind::ChatGpt {
                vec![String::new()]
            } else {
                profile.models.clone()
            };
            for model in models {
                options.push((
                    format!("{}:{model}", profile.id),
                    format!(
                        "{} · {}",
                        profile.name,
                        if model.is_empty() {
                            "默认模型"
                        } else {
                            &model
                        }
                    ),
                ));
            }
        }
        options
    }

    pub(super) fn commit_model_value(&self) -> String {
        let settings = &self.history.commit_message;
        settings
            .provider_id
            .map(|id| format!("{id}:{}", settings.model))
            .unwrap_or_else(|| "inherit".to_owned())
    }

    pub(super) fn commit_model_label(&self) -> String {
        let settings = &self.history.commit_message;
        let Some(profile) = settings
            .provider_id
            .and_then(|id| self.catalog.providers.iter().find(|p| p.id == id))
        else {
            return "跟随当前模型".to_owned();
        };
        if settings.model.is_empty() {
            profile.name.clone()
        } else {
            format!("{} · {}", profile.name, settings.model)
        }
    }

    pub(super) fn choose_commit_model(&mut self, value: &str, cx: &mut Context<Self>) {
        if value == "inherit" {
            self.history.commit_message.provider_id = None;
            self.history.commit_message.model.clear();
        } else if let Some((id, model)) = value.split_once(':')
            && let Ok(id) = Uuid::parse_str(id)
        {
            self.history.commit_message.provider_id = Some(id);
            self.history.commit_message.model = model.to_owned();
        }
        self.save(cx);
        cx.notify();
    }

    /// Runs the request in the background and writes the message into `entry`
    /// once it arrives. `done` clears the caller's own busy handle.
    pub(super) fn spawn_commit_message<T: 'static>(
        &self,
        root: PathBuf,
        entry: Entity<PromptEditor>,
        config: provider::Config,
        prompt: String,
        cx: &mut Context<T>,
        done: impl Fn(&mut T, PathBuf, Result<String, String>, &mut Context<T>) + 'static,
    ) -> Task<()> {
        let previous = entry.read(cx).value().to_owned();
        let finished = root.clone();
        let request = cx.background_executor().spawn(async move {
            let diff = workspace_diff::commit_context(&root)?;
            let key = if config.kind == provider::Kind::ChatGpt {
                String::new()
            } else {
                config.key()?
            };
            let messages = vec![
                serde_json::json!({ "role": "system", "content": prompt }),
                serde_json::json!({ "role": "user", "content": diff }),
            ];
            let completion =
                provider::Request::start(config, key, messages, provider::Thinking::Default)?
                    .response()
                    .await?;
            let message = tidy(&completion.text);
            if message.is_empty() {
                return Err("模型没有返回可用的提交信息".to_owned());
            }
            Ok(message)
        });
        cx.spawn(async move |weak, cx| {
            let result = request.await;
            let _ = weak.update(cx, move |this, cx| {
                if let Ok(message) = &result {
                    // Never overwrite something the user typed meanwhile.
                    entry.update(cx, |entry, cx| {
                        if entry.value() == previous {
                            entry.set_value(message.clone(), cx);
                        }
                    });
                }
                done(this, finished, result, cx);
            });
        })
    }
}

/// A quiet model menu for a commit footer: plain label, options above it.
pub(super) fn model_picker<T: 'static>(
    id: &'static str,
    theme: ThemePreset,
    label: String,
    mut options: Vec<(String, String)>,
    current: String,
    open: bool,
    entity: gpui::WeakEntity<T>,
    set_open: impl Fn(&mut T, bool) + Copy + 'static,
    pick: impl Fn(&mut T, &str, &mut Context<T>) + Copy + 'static,
) -> gpui::AnyElement {
    let p = theme.palette();
    let trigger = div()
        .id(SharedString::from(format!("{id}-trigger")))
        .debug_selector(move || format!("{id}-trigger"))
        .h(px(24.))
        .max_w(px(190.))
        .px_1()
        .flex()
        .items_center()
        .gap_1()
        .rounded(px(6.))
        .text_size(px(11.5))
        .text_color(rgb(p.text_muted))
        .cursor_pointer()
        .hover(move |style| style.text_color(rgb(p.accent)))
        .child(div().min_w_0().truncate().child(label))
        .child(Icon::disclosure(11., open));
    let weak = entity;
    let weak_rows = weak.clone();
    let weak_open = weak.clone();
    let mut rows = div()
        .id(SharedString::from(format!("{id}-options")))
        .flex()
        .flex_col()
        .gap(px(1.))
        .max_h(px(280.))
        .overflow_y_scroll();
    // Rows are rebuilt every frame, so the options can be consumed in place.
    for (index, (value, option)) in std::mem::take(&mut options).into_iter().enumerate() {
        let selected = value == current;
        let weak = weak_rows.clone();
        rows = rows.child(
            div()
                .id(SharedString::from(format!("{id}-option-{index}")))
                .h(px(30.))
                .flex_shrink_0()
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(rgb(if selected { p.accent } else { p.text }))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(p.accent_tint)))
                .child(
                    div()
                        .w(px(14.))
                        .flex_shrink_0()
                        .children(selected.then(|| Icon::CheckCircle.render(12.))),
                )
                .child(div().min_w_0().truncate().child(option))
                .on_click(move |_, _, cx: &mut App| {
                    let _ = weak.update(cx, |this, cx| {
                        pick(this, &value, cx);
                        set_open(this, false);
                        cx.notify();
                    });
                }),
        );
    }
    Popover::new(SharedString::from(format!("{id}-menu")), trigger, rows)
        .open(open)
        .above_trigger()
        .trigger_height(24.)
        .width(260.)
        .padding(4.)
        .theme(theme)
        .on_open_change(move |open, _, cx: &mut App| {
            let _ = weak_open.update(cx, |this, cx| {
                set_open(this, open);
                cx.notify();
            });
        })
        .render()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::tidy;

    #[test]
    fn tidy_strips_fences_labels_and_quotes() {
        assert_eq!(tidy("```\n修复登录\n```"), "修复登录");
        assert_eq!(tidy("\u{201c}修复登录\u{201d}\n"), "修复登录");
        assert_eq!(tidy("提交信息：修复登录\n"), "修复登录");
        assert_eq!(tidy("\n\n修复登录\n\n- 细节\n"), "修复登录\n\n- 细节");
    }
}
