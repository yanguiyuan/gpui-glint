#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod activity_ui;
mod app_icon;
mod background_tasks_ui;
mod collection_tree_ui;
mod collection_ui;
mod commands;
mod curl;
mod curl_ui;
mod directory_ui;
mod environment;
mod environment_ui;
mod history_ui;
mod http;
mod json_picker;
mod mcp;
mod mcp_history;
mod mcp_history_ui;
mod mcp_tools;
mod mcp_ui;
mod model;
mod oauth;
mod oauth_ui;
mod pair_ui;
mod plugin_ui;
mod plugins;
mod post_actions;
mod proxy_capture_ui;
mod proxy_listener_ui;
mod proxy_service;
mod proxy_ui;
mod registry_ui;
mod request_config_ui;
mod request_editor;
mod request_execution;
mod request_fingerprint;
mod request_preview_ui;
mod request_window;
mod response_ui;
mod shell_ui;
mod sse;
mod storage;
mod template;
mod theme_ui;
mod tree_delete_ui;
mod triggers;
mod ui_helpers;
mod update_ui;
mod updater;
mod version_diff;
mod version_history;
mod version_tags;
mod websocket;
mod websocket_ui;
mod workbench;
mod workspace_transfer;
mod workspace_transfer_ui;
mod workspace_ui;

use ui_helpers::{method_color, record_time, status_label, tab_button};
use workbench::{GroupEdit, PairInput, TabCloseMode, TraceEntry, Workbench};

use environment::EnvironmentVariables;
use environment_ui::{ProxyBindingInput, VariableInput};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight,
    KeyBinding, Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img,
    point, prelude::*, px, rgb, size,
};
use gpui_glint::{
    components::{
        AppLayout, Button, ButtonKind, ButtonSize, Checkbox, ClosableTab, ClosableTabs, CodeCard,
        CommandItem, CommandPalette, CommandPaletteEvent, DataTable, Dialog, DropdownMenu,
        GlintAssets, Icon, IconButton, MenuItem, Popover, SearchInput, SegmentOption,
        SegmentedControl, Select, SelectEvent, SelectField, SelectFieldEvent, SelectOption, Switch,
        TableCell, TableColumn, TableEvent, TableRow, TextCompletion, TextEntry,
        TextReferenceResolver, Toast, ToastHost, ToastKind, TreeDrop, TreeListItem,
        bind_code_card_keys, bind_text_entry_keys,
    },
    theme::ThemePreset,
};
use http::{HttpResponse, PreparedRequest};
use model::{BodyFormat, Draft, Pair, Record, Request, RequestGroups};
use proxy_ui::ProxyInput;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

gpui::actions!(
    api_workbench,
    [SendRequest, SaveRequest, SearchCommands, SearchResponse]
);

fn main() {
    if let Some(action) = std::env::args().nth(1) {
        if action == "--mcp" || action == "--mcp-config" {
            let result = if action == "--mcp" {
                mcp::proxy()
            } else {
                mcp::configuration().map(|config| println!("{}", config))
            };
            if let Err(error) = result {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--plugin-worker")) {
        if !plugins::worker() {
            std::process::exit(1);
        }
        return;
    }
    let mut args = std::env::args_os().skip(1);
    let action = args.next();
    if action.as_deref() == Some(std::ffi::OsStr::new("--version")) {
        println!("Glint API {}", updater::VERSION);
        return;
    }
    if action.as_deref() == Some(std::ffi::OsStr::new("--check-update")) {
        match updater::check_blocking() {
            Ok(update) => println!(
                "{}",
                serde_json::json!({
                    "currentVersion": updater::VERSION,
                    "availableVersion": update.as_ref().map(|update| update.version.to_string()),
                    "releaseUrl": update.as_ref().map(updater::Update::release_url),
                })
            ),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if action.as_deref() == Some(std::ffi::OsStr::new("--download-release")) {
        let result = args
            .next()
            .and_then(|value| value.into_string().ok())
            .ok_or("缺少发布版本".to_owned())
            .and_then(|version| {
                args.next()
                    .ok_or("缺少下载目录".to_owned())
                    .and_then(|directory| {
                        updater::download_release(&version, std::path::Path::new(&directory))
                    })
            });
        match result {
            Ok(path) => println!("{}", path.display()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if action.as_deref() == Some(std::ffi::OsStr::new("--apply-update")) {
        let result = args
            .next()
            .ok_or("缺少更新任务路径".to_owned())
            .and_then(|path| updater::run_helper(std::path::Path::new(&path)));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = updater::ensure_not_installing() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    let _instance_guard = match mcp::workbench_guard() {
        Ok(Some(guard)) => guard,
        Ok(None) => return,
        Err(error) => {
            eprintln!("无法锁定 Glint API 工作台：{error}");
            std::process::exit(1);
        }
    };
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            app_icon::install();
            bind_text_entry_keys(cx);
            bind_code_card_keys(cx);
            cx.bind_keys([
                KeyBinding::new(
                    "cmd-enter",
                    SendRequest,
                    Some("ApiWorkbench > GlassCodeCard"),
                ),
                KeyBinding::new(
                    "ctrl-enter",
                    SendRequest,
                    Some("ApiWorkbench > GlassCodeCard"),
                ),
                KeyBinding::new("cmd-enter", SendRequest, Some("ApiWorkbench")),
                KeyBinding::new("ctrl-enter", SendRequest, Some("ApiWorkbench")),
                KeyBinding::new("cmd-s", SaveRequest, Some("ApiWorkbench")),
                KeyBinding::new("ctrl-s", SaveRequest, Some("ApiWorkbench")),
                KeyBinding::new("cmd-f", SearchResponse, Some("ApiWorkbench")),
                KeyBinding::new("ctrl-f", SearchResponse, Some("ApiWorkbench")),
                KeyBinding::new("cmd-k", SearchCommands, Some("ApiWorkbench")),
                KeyBinding::new("ctrl-k", SearchCommands, Some("ApiWorkbench")),
            ]);
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1050.), px(700.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Glint API".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(14.))),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let app = cx.new(Workbench::new);
                    let closing = app.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        let _ = closing.update(cx, |app, cx| app.persist(cx));
                        true
                    });
                    let focus = app.read(cx).focus.clone();
                    window.focus(&focus, cx);
                    app
                },
            )
            .expect("无法打开 API 工作台窗口");
            cx.activate(true);
        });
}

#[cfg(test)]
mod collection_rename_tests;
#[cfg(test)]
mod persistence_tests;
