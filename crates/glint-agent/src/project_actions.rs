//! User-triggered, project-scoped commands. Discovery never evaluates Makefiles.
use super::*;
use gpui_glint::components::{Dialog, InputField, MenuItem, Tooltip};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Action {
    name: String,
    command: String,
    #[serde(default)]
    target: Option<String>,
}
#[derive(Default, Clone, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    projects: BTreeMap<PathBuf, Vec<Action>>,
}
struct Run {
    action: Action,
    terminal: Entity<terminal_ui::TerminalView>,
}
pub struct State {
    open: bool,
    pub(super) panel: bool,
    root: Option<PathBuf>,
    discovered: Vec<Action>,
    discovery: Option<Task<()>>,
    error: Option<String>,
    config: Config,
    path: PathBuf,
    before: Option<String>,
    readable: bool,
    runs: HashMap<PathBuf, Run>,
    pub(super) editor: Option<(PathBuf, Option<usize>)>,
    name: Entity<TextEntry>,
    command: Entity<TextEntry>,
}
impl State {
    pub fn new(anchor: &Path, cx: &mut Context<AgentWorkspace>) -> Self {
        let path = anchor.with_file_name("project-actions.json");
        let loaded = atomic_file::current(&path).and_then(|text| {
            let config = text
                .as_deref()
                .map(serde_json::from_str::<Config>)
                .transpose()
                .map_err(|e| format!("项目操作配置无效：{e}"))?
                .unwrap_or_default();
            Ok((text, config))
        });
        let (before, config, error, readable) = match loaded {
            Ok((text, config)) => (text, config, None, true),
            Err(error) => (None, Config::default(), Some(error), false),
        };
        Self {
            open: false,
            panel: false,
            root: None,
            discovered: vec![],
            discovery: None,
            error,
            config,
            path,
            before,
            readable,
            runs: HashMap::new(),
            editor: None,
            name: cx.new(|cx| TextEntry::new(cx, "例如：启动开发", "")),
            command: cx.new(|cx| TextEntry::new(cx, "例如：cargo run -p glint-workbench", "")),
        }
    }
    fn save(&mut self, config: Config) -> Result<(), String> {
        if !self.readable {
            return Err("配置读取失败，未覆盖原文件".into());
        }
        let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
        let parent = self.path.parent().ok_or("配置目录无效")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let path = parent
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join("project-actions.json");
        atomic_file::replace(&path, self.before.as_deref(), &text)?;
        self.before = Some(text);
        self.config = config;
        Ok(())
    }
}

fn make_actions(source: &str) -> Vec<Action> {
    let mut seen = HashSet::new();
    let mut actions = vec![];
    let mut define_depth = 0usize;
    let mut continuation = false;
    for line in source.lines() {
        let continued = continuation;
        continuation = line.trim_end().ends_with('\\');
        if continued || line.starts_with('\t') {
            continue;
        }
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with("define ") {
            define_depth += 1;
            continue;
        }
        if line == "endef" {
            define_depth = define_depth.saturating_sub(1);
            continue;
        }
        if define_depth > 0 {
            continue;
        }
        let Some((targets, rest)) = line.split_once(':') else {
            continue;
        };
        if targets.contains(['=', '$', '%', '\\']) || rest.starts_with('=') {
            continue;
        }
        for target in targets.split_whitespace() {
            // Deliberately offer only literal, shell-safe targets. Computed and
            // pattern rules cannot be discovered without evaluating make code.
            if target.starts_with(['.', '-'])
                || !target
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_./-".contains(c))
            {
                continue;
            }
            if seen.insert(target.to_string()) {
                actions.push(Action {
                    name: target.into(),
                    command: format!("make {target}"),
                    target: Some(target.into()),
                });
            }
        }
    }
    actions
}

async fn discover(root: PathBuf) -> Result<Vec<Action>, String> {
    const NAMES: [&str; 3] = ["GNUmakefile", "makefile", "Makefile"];
    if ssh_project::is_remote(&root) {
        let remote = ssh_project::Remote::from_path(&root)?;
        // Probe names via read-only file operations; never invoke make, even -n.
        for name in NAMES {
            let first = remote.action("read_file", serde_json::json!({"path":name,"line_numbers":false,"include_metadata":true,"lines":500}), false).await;
            let first = match first {
                Ok(value) => value,
                Err(error) if error.contains("No such file") || error.contains("不存在") => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            let mut text = first["content"].as_str().unwrap_or_default().to_string();
            let mut next = first["next_line"].as_u64();
            while let Some(start) = next {
                if text.len() > 2 * 1024 * 1024 || start > 20000 {
                    return Err("Makefile 过大，未完成识别".into());
                }
                let page = remote.action("read_file", serde_json::json!({"path":name,"line_numbers":false,"include_metadata":true,"lines":500,"start_line":start}), false).await?;
                text.push_str(page["content"].as_str().unwrap_or_default());
                next = page["next_line"].as_u64();
            }
            return Ok(make_actions(&text));
        }
        return Ok(vec![]);
    }
    tokio::task::spawn_blocking(move || {
        for name in NAMES {
            let path = root.join(name);
            match std::fs::metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
                Ok(meta) if meta.len() > 2 * 1024 * 1024 => {
                    return Err("Makefile 超过 2MB，未识别".into());
                }
                Ok(_) => {
                    return std::fs::read_to_string(path)
                        .map(|text| make_actions(&text))
                        .map_err(|e| e.to_string());
                }
            }
        }
        Ok(vec![])
    })
    .await
    .map_err(|e| e.to_string())?
}

fn action_command(action: &Action) -> String {
    action
        .target
        .as_ref()
        .map(|target| format!("make -- {target}"))
        .unwrap_or_else(|| action.command.clone())
}

impl AgentWorkspace {
    pub(super) fn sync_project_actions(&mut self, cx: &mut Context<Self>) {
        for run in self.project_actions.runs.values() {
            run.terminal
                .update(cx, |view, cx| view.set_theme(self.theme, cx));
        }
        if self.project_actions.open && self.project_actions.root != self.history.active().project {
            self.refresh_project_actions(cx);
        }
    }
    fn refresh_project_actions(&mut self, cx: &mut Context<Self>) {
        let root = self.history.active().project.clone();
        self.project_actions.root = root.clone();
        self.project_actions.discovered.clear();
        self.project_actions.discovery = None;
        if self.project_actions.readable {
            self.project_actions.error = None;
        }
        let Some(root) = root else {
            cx.notify();
            return;
        };
        match provider::Job::start(discover(root.clone())) {
            Err(e) => self.project_actions.error = Some(e),
            Ok(job) => {
                self.project_actions.discovery = Some(cx.spawn(async move |weak, cx| {
                    let result = job.result().await;
                    let _ = weak.update(cx, |this, cx| {
                        if this.project_actions.root.as_ref() != Some(&root) {
                            return;
                        }
                        if let Some(task) = this.project_actions.discovery.take() {
                            task.detach();
                        }
                        match result {
                            Ok(actions) => this.project_actions.discovered = actions,
                            Err(e) => this.project_actions.error = Some(e),
                        }
                        cx.notify();
                    });
                }))
            }
        }
        cx.notify();
    }
    fn run_project_action(
        &mut self,
        root: PathBuf,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .project_actions
            .runs
            .get(&root)
            .is_some_and(|r| r.terminal.read(cx).busy())
        {
            return;
        }
        let command = action_command(&action);
        let terminal = cx
            .new(|cx| terminal_ui::TerminalView::new_action(root.clone(), command, self.theme, cx));
        cx.subscribe(&terminal, |_, _, _: &terminal_ui::StatusChanged, cx| {
            cx.notify()
        })
        .detach();
        window.focus(&terminal.read(cx).focus_handle(cx), cx);
        self.project_actions
            .runs
            .insert(root, Run { action, terminal });
        self.project_actions.open = false;
        self.project_actions.panel = true;
        if self.terminal_ui.bottom {
            self.terminal_ui.open = false;
        }
        self.page = crate::Page::Chat;
        cx.notify();
    }
    fn edit_project_action(
        &mut self,
        root: PathBuf,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = index
            .and_then(|i| self.project_actions.config.projects.get(&root)?.get(i))
            .cloned();
        self.project_actions.name.update(cx, |input, cx| {
            input.set_theme(self.theme, cx);
            input.set_value(action.as_ref().map_or("", |a| a.name.as_str()), cx);
        });
        self.project_actions.command.update(cx, |input, cx| {
            input.set_theme(self.theme, cx);
            input.set_value(action.as_ref().map_or("", |a| a.command.as_str()), cx);
        });
        self.project_actions.editor = Some((root, index));
        self.project_actions.open = false;
        window.focus(&self.project_actions.name.focus_handle(cx), cx);
        cx.notify();
    }
    fn save_project_action(&mut self, cx: &mut Context<Self>) {
        let Some((root, index)) = self.project_actions.editor.clone() else {
            return;
        };
        let name = self
            .project_actions
            .name
            .read(cx)
            .value()
            .trim()
            .to_string();
        let command = self
            .project_actions
            .command
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() || command.is_empty() || command.contains(['\0', '\n', '\r']) {
            self.project_actions.error = Some("请填写名称和单行命令".into());
            cx.notify();
            return;
        }
        let mut config = self.project_actions.config.clone();
        let actions = config.projects.entry(root).or_default();
        let action = Action {
            name,
            command,
            target: None,
        };
        if let Some(index) = index {
            actions[index] = action;
        } else {
            actions.push(action);
        }
        match self.project_actions.save(config) {
            Ok(()) => {
                self.project_actions.editor = None;
                self.project_actions.error = None;
            }
            Err(e) => self.project_actions.error = Some(e),
        }
        cx.notify();
    }
    pub(super) fn project_action_trigger(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let root = self.history.active().project.clone();
        let busy = root
            .as_ref()
            .and_then(|r| self.project_actions.runs.get(r))
            .is_some_and(|r| r.terminal.read(cx).busy());
        let trigger = div()
            .debug_selector(|| "agent-topbar-actions".into())
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded(px(5.))
            .text_color(rgb(p.accent))
            .bg(rgb(p.accent_tint))
            .child(
                if busy { Icon::Pulse } else { Icon::Play }
                    .render(15.)
                    .when(!busy, |icon| icon.text_color(rgb(self.theme.color(0x16833a)))),
            )
            .child(if busy { "运行中" } else { "项目操作" })
            .child(Icon::ChevronDown.render(12.));
        let mut content = div().flex().flex_col().min_w_0();
        if let Some(root) = root {
            content = content.child(
                Tooltip::new(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .pt_1()
                        .pb_2()
                        .text_color(rgb(p.text_muted))
                        .child(Icon::Folder.render(14.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(ssh_project::label(&root)),
                        )
                        .when(busy, |header| {
                            header.child(
                                div()
                                    .px_1()
                                    .rounded(px(4.))
                                    .text_size(px(10.))
                                    .text_color(rgb(p.accent))
                                    .bg(rgb(p.accent_tint))
                                    .child("运行中"),
                            )
                        }),
                    root.to_string_lossy().into_owned(),
                )
                .theme(self.theme)
                .render(cx),
            );
            let mut actions = self
                .project_actions
                .config
                .projects
                .get(&root)
                .cloned()
                .unwrap_or_default();
            let custom_count = actions.len();
            if self.project_actions.root.as_ref() == Some(&root) {
                actions.extend(self.project_actions.discovered.clone());
            }
            let mut list = div()
                .id("project-actions-list")
                .max_h(px(320.))
                .overflow_y_scroll()
                .flex()
                .flex_col();
            for (index, action) in actions.into_iter().enumerate() {
                if index == 0 && custom_count > 0 {
                    list = list.child(
                        div()
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("自定义 · {custom_count}")),
                    );
                }
                if index == custom_count {
                    list = list.child(
                        Tooltip::new(
                            div()
                                .px_2()
                                .pt_2()
                                .pb_1()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(format!(
                                    "Makefile · {}",
                                    self.project_actions.discovered.len()
                                )),
                            "识别根目录 Makefile 中的静态目标；不展开 include、变量和模式规则",
                        )
                        .theme(self.theme)
                        .render(cx),
                    );
                }
                let project = root.clone();
                let edit_root = root.clone();
                let delete_root = root.clone();
                let row = div()
                    .flex()
                    .flex_shrink_0()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id(SharedString::from(format!("project-action-{index}")))
                            .debug_selector(move || format!("project-action-{index}"))
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(p.surface_subtle)))
                            .child(
                                Icon::Play
                                    .render(13.)
                                    .text_color(rgb(self.theme.color(0x16833a))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(action.name.clone()),
                                    )
                                    .child(
                                        Tooltip::new(
                                            div()
                                                .w_full()
                                                .truncate()
                                                .text_size(px(10.))
                                                .text_color(rgb(p.text_muted))
                                                .child(action.command.clone()),
                                            action.command.clone(),
                                        )
                                        .theme(self.theme)
                                        .render(cx)
                                        .min_w_0()
                                        .w_full(),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_project_action(project.clone(), action.clone(), window, cx)
                            })),
                    )
                    .when(index < custom_count, |row| {
                        row.child(
                            IconButton::new(Icon::Pencil, "编辑操作")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .size(px(24.))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_project_action(
                                        edit_root.clone(),
                                        Some(index),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            IconButton::new(Icon::Trash, "删除操作")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .size(px(24.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut config = this.project_actions.config.clone();
                                    if let Some(actions) = config.projects.get_mut(&delete_root) {
                                        if index < actions.len() {
                                            actions.remove(index);
                                        }
                                    }
                                    if let Err(e) = this.project_actions.save(config) {
                                        this.project_actions.error = Some(e);
                                    }
                                    cx.notify();
                                })),
                        )
                    });
                list = list.child(row);
            }
            content = content.child(list);
            if self.project_actions.discovery.is_some() {
                content = content.child(
                    div()
                        .px_2()
                        .py_3()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child("正在识别 Makefile…"),
                );
            } else if custom_count == 0 && self.project_actions.discovered.is_empty() {
                content = content.child(
                    div()
                        .px_2()
                        .py_4()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("暂无操作，添加命令或在项目根目录创建 Makefile"),
                );
            }
            content = content.child(
                div()
                    .mt_2()
                    .pt_1()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div().flex_1().min_w_0().child(
                            MenuItem::new("add-project-action", "添加操作…")
                                .icon(Icon::Plus)
                                .theme(self.theme)
                                .render()
                                .debug_selector(|| "add-project-action".into())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_project_action(root.clone(), None, window, cx)
                                })),
                        ),
                    )
                    .child(
                        Tooltip::new(
                            IconButton::new(Icon::Refresh, "刷新 Makefile")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .debug_selector(|| "refresh-project-actions".into())
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.refresh_project_actions(cx)),
                                ),
                            "刷新 Makefile 操作",
                        )
                        .theme(self.theme)
                        .render(cx),
                    ),
            );
            if self
                .history
                .active()
                .project
                .as_ref()
                .is_some_and(|r| self.project_actions.runs.contains_key(r))
            {
                content = content.child(
                    MenuItem::new("show-project-action-output", "显示运行输出")
                        .icon(Icon::Shell)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.project_actions.panel = true;
                            if this.terminal_ui.bottom {
                                this.terminal_ui.open = false;
                            }
                            this.project_actions.open = false;
                            this.page = crate::Page::Chat;
                            cx.notify();
                        })),
                );
            }
        } else {
            content = content.child(div().p_2().child("请先打开项目"));
        }
        content = content.when_some(self.project_actions.error.clone(), |body, e| {
            body.child(div().p_2().text_color(rgb(0xd9342b)).child(e))
        });
        let weak = cx.entity().downgrade();
        Popover::new("project-actions", trigger, content)
            .open(self.project_actions.open)
            .width(360.)
            .padding(6.)
            .theme(self.theme)
            .on_open_change(move |open, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.project_actions.open = open;
                    if open {
                        this.refresh_project_actions(cx);
                    }
                    cx.notify();
                });
            })
    }
    pub(super) fn project_action_editor(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let root = self
            .project_actions
            .editor
            .as_ref()
            .map(|e| ssh_project::label(&e.0))
            .unwrap_or_default();
        let shell = "项目终端默认 shell";
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(18.)).child("自定义项目操作"))
            .child(
                div()
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(format!("{root} · {shell} · 在项目根目录执行")),
            )
            .child(
                InputField::new("操作名称")
                    .label("操作名称")
                    .control(self.project_actions.name.clone())
                    .theme(self.theme)
                    .render(),
            )
            .child(
                InputField::new("命令")
                    .label("命令")
                    .control(self.project_actions.command.clone())
                    .theme(self.theme)
                    .render(),
            )
            .when_some(self.project_actions.error.clone(), |body, e| {
                body.child(div().text_color(rgb(0xd9342b)).child(e))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Plain)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_actions.editor = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("保存")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.save_project_action(cx))),
                    ),
            );
        let weak = cx.entity().downgrade();
        Dialog::new("project-action-editor", body)
            .width(560.)
            .theme(self.theme)
            .focus_handles([
                self.project_actions.name.focus_handle(cx),
                self.project_actions.command.focus_handle(cx),
            ])
            .on_close(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.project_actions.editor = None;
                    cx.notify();
                });
            })
            .render()
    }
    pub(super) fn project_action_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let root = self.history.active().project.clone();
        let run = root.as_ref().and_then(|r| self.project_actions.runs.get(r));
        let action = run.map(|r| r.action.clone());
        let terminal = run.map(|r| r.terminal.clone());
        let busy = terminal.as_ref().is_some_and(|t| t.read(cx).busy());
        let status = terminal
            .as_ref()
            .map(|t| t.read(cx).status())
            .unwrap_or_else(|| "当前项目暂无运行记录".into());
        let title = format!(
            "{} · {status}",
            action.as_ref().map_or("项目操作", |a| a.name.as_str())
        );
        let completed = terminal
            .as_ref()
            .is_some_and(|terminal| terminal.read(cx).completed_successfully());
        let title = div()
            .flex()
            .items_center()
            .gap_2()
            .when(completed, |title| {
                title.child(
                    Icon::CheckCircle
                        .render(16.)
                        .flex_shrink_0()
                        .text_color(rgb(self.theme.color(0x16833a))),
                )
            })
            .child(div().min_w_0().truncate().child(title));
        let controls = div()
            .flex()
            .items_center()
            .gap_2()
            .when(busy, |bar| {
                bar.child(
                    IconButton::new(Icon::Pause, "停止操作")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(run) = this
                                .history
                                .active()
                                .project
                                .as_ref()
                                .and_then(|r| this.project_actions.runs.get(r))
                            {
                                run.terminal.update(cx, |view, cx| view.stop(cx));
                            }
                            cx.notify();
                        })),
                )
            })
            .when(!busy && action.is_some(), |bar| {
                bar.child(
                    IconButton::new(Icon::Refresh, "重跑操作")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let (Some(root), Some(action)) = (root.clone(), action.clone()) {
                                this.run_project_action(root, action, window, cx);
                            }
                        })),
                )
            })
            .child(
                IconButton::new(Icon::Copy, "复制输出")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(run) = this
                            .history
                            .active()
                            .project
                            .as_ref()
                            .and_then(|r| this.project_actions.runs.get(r))
                        {
                            run.terminal.update(cx, |view, cx| view.copy_output(cx));
                        }
                    })),
            )
            .child(
                IconButton::new(Icon::Close, "收起输出（继续运行）")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_actions.panel = false;
                        cx.notify();
                    })),
            );
        terminal_ui::terminal_panel(
            terminal,
            title,
            controls,
            self.theme,
            true,
            self.right_sidebar_width,
        )
        .debug_selector(|| "project-action-output".into())
        .border_color(rgb(p.border))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_literal_rules_without_recipes_assignments_or_patterns() {
        let source = "VAR := value\n.PHONY: test clean\nall test: dep\n\techo bad: fake\nclean::\n%.o: %.c\n$(DYNAMIC):\n$(shell echo injected):\nfoo bar &: dep\ndefine SCRIPT\nnot-a-target: text\nendef\nnext:; echo okay\n构建:\n";
        let targets: Vec<_> = make_actions(source).into_iter().map(|a| a.name).collect();
        assert_eq!(
            targets,
            ["all", "test", "clean", "foo", "bar", "next", "构建"]
        );
    }
    #[tokio::test]
    async fn discovery_obeys_make_filename_precedence_and_never_executes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "lower:\n").unwrap();
        std::fs::write(
            dir.path().join("GNUmakefile"),
            "$(shell touch SHOULD_NOT_EXIST)\nhigher:\n",
        )
        .unwrap();
        let actions = discover(dir.path().into()).await.unwrap();
        assert_eq!(actions[0].name, "higher");
        assert_eq!(actions.len(), 1);
        assert!(!dir.path().join("SHOULD_NOT_EXIST").exists());
    }

    #[gpui::test]
    fn topbar_opens_actions_and_custom_commands_persist_per_project(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) =
            cx.add_window_view(|_, cx| AgentWorkspace::with_path(path.clone(), cx));
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |host, cx| {
            host.history.active_mut().project = Some(dir.path().into());
            cx.notify();
        });
        window.run_until_parked();
        let trigger = window.debug_bounds("agent-topbar-actions").unwrap();
        window.simulate_click(trigger.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| assert!(host.project_actions.open));
        let add = window.debug_bounds("add-project-action").unwrap();
        window.simulate_click(add.center(), Default::default());
        window.run_until_parked();
        host.update(window, |host, cx| {
            host.project_actions
                .name
                .update(cx, |input, cx| input.set_value("检查", cx));
            host.project_actions
                .command
                .update(cx, |input, cx| input.set_value("cargo check", cx));
            host.save_project_action(cx);
            assert!(
                host.project_actions.editor.is_none(),
                "{:?}",
                host.project_actions.error
            );
            assert_eq!(
                host.project_actions.config.projects[dir.path()][0].command,
                "cargo check"
            );
            assert!(
                !host
                    .project_actions
                    .config
                    .projects
                    .contains_key(&dir.path().join("other"))
            );
        });
        let loaded: Config = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("project-actions.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(loaded.projects[dir.path()][0].name, "检查");
        // External changes must not be overwritten by a stale editor.
        std::fs::write(dir.path().join("project-actions.json"), "external change").unwrap();
        host.update(window, |host, _| {
            assert!(host.project_actions.save(Config::default()).is_err());
            assert_eq!(host.project_actions.config.projects[dir.path()].len(), 1);
        });
    }
}
