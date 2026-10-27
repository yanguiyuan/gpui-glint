# Changelog

## Unreleased

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
