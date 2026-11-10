# Changelog

## Unreleased

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
