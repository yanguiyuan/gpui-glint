# Changelog

## Unreleased

- Load data-only TextMate JSON language packs so file previews, Markdown code fences and diff cards highlight DSLs that the built-in lexers do not cover. A pack is a directory under `<agent data directory>/syntax/` (`GLINT_SYNTAX_DIR` overrides it) holding a manifest that declares `api_version` and up to 128 `syntax.language` contributions, each naming a grammar of at most 512 KiB plus aliases, extensions and file names; grammar paths must stay inside the pack and oversized files are truncated by a bounded read instead of loading. Tokens come from `irosashi` without evaluating the pack's scripts and are cached for the process, so installing or removing a pack takes effect after a restart. An installed language wins over the built-in lexer and falls back to it when the grammar cannot tokenize a document. `docs/language-packs.md` documents the format and ships a runnable JSON example in `examples/language-packs/json`.
- Show the Agent Git workspace inventory as either a flat list or a tree grouped by folder: a directory row counts the files below it, collapses independently, and leaves the diff and whitespace-fold state of its files intact, while tree rows reuse the existing virtualization for large change sets. The chosen mode persists as `workspace_display` with the session history.
- Replace the 窗口外观 and 文件展示形态 button pairs in 设置 with the shared `SegmentedControl`: the raised thumb follows the theme category, so any light preset selects 浅色 and any dark preset selects 深色, the caption names the current theme, and switching still updates every themable panel or saves the display mode. `SegmentOption::debug_selector` exposes each segment's bounds to UI tests.
- Rework the project group dialog into explicit modes: it opens as 自动跟随分支 when the project has no custom name and as 手动设置 otherwise, the manual field accepts an empty name for no grouping, existing group names appear as quick picks that highlight the current value, and nothing is persisted until 保存. The project menu gains 移除项目, which drops only the list entry — files, worktrees and history stay, and reopening the project restores it — and records the path in a persisted `removed_projects` set so the project's still-open session cannot add it back.
- Let the 打开项目终端 shortcut be recorded from 设置 → 快捷键: clicking the keycap captures one combination (Esc cancels, lone modifier keys are ignored), combinations without Ctrl/Alt/⌘ or already bound to an app or editing command are rejected with an inline message, and a valid one is stored in the session history and shown as keycaps in the shortcuts page and the command palette. 恢复默认 restores the default Ctrl + backtick binding, and histories saved before the field existed fall back to it.
- Redesign the Agent question panel: each question renders as a bounded card with numbered option rows that answer on click, the free-form field and 提交回答 / 跳过 sit inside the card, and long option text wraps instead of widening the panel or displacing the composer. The panel is now body content and only appears while a question is unanswered.
- Let the Agent use the vault without exposing secrets: add the read-only `vault_list_entries` tool (id, title, username, website, environment identifier and `GLINT_*_PWD` variable name; `query` / `offset` / `limit`, and an explicit locked state) plus an optional per-entry environment identifier that is uppercased, restricted to letters, digits and `_`, unique, and exported as `GLINT_<IDENTIFIER>_PWD` to newly started local `run_command` processes and project terminals. SSH commands are never injected, and editing, deleting, locking or exiting drops the copy so the next process sees the new snapshot while already running processes keep theirs and must be restarted. Passwords and notes are never returned to the model, and the tool is read-only in Code Mode.
- Group Agent sessions by worktree branch: projects that are linked worktrees nest under one parent group named after the branch, with the repository name on the child row, and each level collapses independently. Search matches branch and repository names, and a new project-group dialog (the gear on a group header or 设置当前项目分组… in the project menu) assigns a custom name, clears grouping, or restores 跟随 worktree 分支. Metadata is probed from Git in the background, persisted with the other project state, and saved labels survive offline or failed probes.
- Mark a finished Agent project action as successful: the action rows and the 项目操作 trigger use a green play icon, and the terminal panel title gains a check badge once the command exits with status 0, while stopped, failed or still running commands keep the plain title.
- Accept verbatim Windows paths when building the installer: `build-windows.ps1` strips the `\\?\` / `\\?\UNC\` prefix before resolving its own directory, and `test-packaging.ps1` plans both forms to prove stage, output, installer, icon and template paths are identical.
- Add “创建 worktree 分支” to the Agent branch menu: the dialog previews the destination (`<main project parent>/worktree/<project>/<branch>`, also for SSH projects), validates the name with `git check-ref-format`, refuses existing branches, then creates the worktree at `HEAD` and opens its session. Existing worktrees are listed in the same menu and open directly; deletion asks for confirmation and removes only the working directory and its registration — the branch and history survive. The main and current directories, locked worktrees, and worktrees with uncommitted or untracked changes are refused, and stale rows are rejected after a refresh.
- Rework the Agent project-actions menu: the header shows a folder icon with the truncated project path in a tooltip plus a 运行中 badge, actions are grouped under 自定义 / Makefile headers (the latter explains that only static root-Makefile targets are discovered), and each row pairs a play icon with the action name and its command in a tooltip.
- Add a binary request-body mode to the API workbench: pick or clear one local file (≤16 MiB) and send its raw bytes on POST/PUT/PATCH/DELETE, defaulting to `application/octet-stream` unless Content-Type is explicit. The file travels with drafts, versions, history, and workspace export; cURL export is disabled for binary bodies and text plugin payloads require valid UTF-8.
- Add collection file variables (`{{upload.name}}`, `{{upload.size}}`, `{{upload.sha256}}`) managed from a table in the environment workspace: one shared file per collection, click to replace, metadata snapshotted at selection, bytes read at send time and shared with prerequisite requests. Exports keep variable names but drop machine-local paths and mark them for re-selection; file names are escaped correctly inside JSON bodies.
- Assign the proxy listener port at bind time when the field is blank or `0`, keep an explicit port when one is given, and stop treating the legacy `8899` default as a configured environment proxy.
- Support WebSocket in the capture proxy: HTTP Upgrade handshakes on absolute `ws://` / `wss://` URLs and CONNECT tunnels that carry either plaintext handshakes or TLS.
- Route proxy browser windows through a stable loopback relay and reuse one managed profile per browser under `proxy-service/browser-profiles`, so history and logins survive proxy and port switches; opening a browser retargets every existing Glint proxy window to the newly selected proxy.
- Re-check an Agent session title after every reply: keep the current title when the model answers `KEEP_TITLE` for follow-ups, refresh it when a new task starts, and cancel pending checks when the next turn begins.
- Add `make dev` for a local run; on Windows it loads the MSVC x64 toolchain through the shared `packaging/windows/msvc.ps1` helper, which the installer build now reuses.
- Prefer image files among clipboard external paths over the embedded bitmap when pasting screenshots, deduplicating paths so one copied image yields one attachment.
- Add `Button::truncate_label()` for long labels and let proxy binding rows wrap instead of clipping.

- Chain wheel scrolling in nested scrollables: the new `ChainScroll` trait stops wheel events at the innermost container and only releases them to outer scroll containers once it is pinned at its top or bottom edge. `DiffCard` bodies and the conflict merge-result editor use it, so scrolling an expanded diff inside the workspace file list no longer scrolls the list in lockstep.
- Let managed-queue 引导 (steering) carry image submissions: queued prompts with images now show the steer control, and steering delivers the text plus prepared images to the running generation (`provider::Steering`) instead of silently dropping them; the in-flight history entry keeps the images.
- Make whitespace-only diff churn readable at a glance in `DiffCard`: align replacement blocks by whitespace-normalized content so inserted/deleted lines do not shift later pairs. Unified diffs merge formatting-only pairs into one neutral gray row with visible whitespace (`·` for spaces, `→` for tabs) and separate old/new line numbers; side-by-side diffs retain both versions. A header pill `隐藏空白 N` treats formatting rows as foldable context with explicit fold counts (raw stats unchanged); hosts with custom headers can drive `set_hide_whitespace` themselves — the workspace file rows expose the same pill.
- Redesign the merge-conflict resolution cards in the Git workspace: each hunk shows ours/theirs as syntax-highlighted read-only previews with diff-style red/green gutter stripes, and the merged result edits in a line-numbered `CodeCard` in the file's language. Conflict rows carry a git-conflict badge with the pristine hunk count, a green banner appears once no markers remain, and notices render as styled pills.
- Turn merge-conflict agent requests into instruction text plus a separate snapshot attachment: chat history shows a compact highlighted `@冲突快照-…` reference with a dedicated git-conflict icon that opens the snapshot in the file preview, while the model still receives the full fenced snapshot. Add `Icon::GitConflict` and `ComposerReference::conflict_snapshot`.
- Add cached, gitignore-aware project filename/directory suggestions to the Agent @panel, with scoped selection and bounded result ranking.
- Add Glint Agent local Skills discovery, settings, progressive `skill_list` / `skill_read` tools, and skill selection from the composer's context panel.
- Add `ComposerEvent::ContextOpened`, dynamic `set_context_sections`, and literal `add_context_token` support while preserving file mentions and image drafts. Filter context rows by caret-scoped @queries and highlight registered $skill tokens, including restored drafts.

- Add opt-in `Composer::stop_on_busy(true)` and `ComposerEvent::StopRequested` to reuse the send button for cancellation.
- Move Glint Agent state to SQLite with legacy JSON import; add scoped semantic memory recall, memory tools, and configurable nightly conversation consolidation.

- Add clipboard image paste to `PromptEditor`: ⌘/Ctrl-V with an image (or copied image files) emits `PromptEditorEvent::ImagePasted` carrying `PastedImage` bytes instead of inserting text.
- Add pasted-image attachments to `Composer` with a removable thumbnail strip; submissions now carry text plus images. **Breaking:** `ComposerEvent::Submitted` and `Queued` hold a `ComposerSubmission { text, images }` instead of a bare `SharedString`.

## 0.1.3 — 2026-10-12

- Add API template methods for random values, UUIDs, names, XSS test text, UTC time, and request Cookie extraction, with per-send evaluation and copyable examples.

- Add controlled `TreeList` containers with hierarchy and ancestor-preserving filtering, reusable `TreeListItem` rows with scoped drag-and-drop, and persist API tree moves and sibling ordering.
- Preserve responses, headers, previews, and traces in new request history records and restore them when selected.
- Allow selecting and copying response header text with read-only `TextEntry` cells; add `Separator` for adjacent panels.
- Add saved, per-environment HTTP/HTTPS proxies with optional TLS certificate verification bypass.
- Add a per-request option to skip TLS certificate verification, persisted with drafts, versions, and request history.
- Create API request versions on save only when the configuration differs from the latest saved snapshot.
- Add configurable single-row empty states to `DataTable`, and use one for Cookie field overrides in the API workbench.
- Replace the API workbench's mandatory group level with a nested directory tree; collections can contain requests directly.
- Add closed/open folder icons, ancestor-preserving search, and contextual dropdown menus for directory actions.
- Migrate legacy groups without losing requests, versions, or history, and persist the new workspace format as database version 2.
- Allow editing request names in the API workbench and add a minimal flat application icon.

## 0.1.2 — 2026-10-12

- Place the brand at the left edge of Windows topbars without reserving space for macOS traffic lights.
- Add native Windows minimize, maximize/restore, and close controls to `AppLayout`, with `.window_controls(false)` for embedded previews.
- Add `window_drag_region()` for topbar dragging and double-click maximize/restore, and integrate it into the playground, Markdown editor, and API workbench.

## 0.1.1 — 2026-10-11

- Fix the API workbench's default SQLite path on Windows to use `LOCALAPPDATA`, with a `USERPROFILE\AppData\Local` fallback, without requiring `HOME`.
- Document Windows database paths and add regression tests for directory selection and missing environment variables.

## 0.1.0 — 2026-10-10

Initial release of Glint UI, a Rust component library for GPUI-CE.

- Light and dark themes, embedded icons, layouts, navigation, and form controls.
- Native text inputs, configurable reference badges, and editable data tables.
- Code cards, Markdown documents, Mermaid diagrams, and conversation components.
- Optional native webviews on macOS and Windows with the `webview` feature.
- Component documentation and standalone playground, Markdown editor, and API workbench examples.
