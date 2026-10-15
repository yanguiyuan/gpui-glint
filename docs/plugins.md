# Reusable plugins

Enable the optional feature; ordinary UI-library consumers do not load Boa:

```toml
[dependencies]
gpui-glint = { path = "../gpui-glint", features = ["plugins"] }
```

```rust,ignore
use gpui_glint::{plugins::{Plugin, PluginRuntime, HostRegistry, Limits, Page}, components::{PluginPage, PluginPageEvent}};
```

The common runtime lives in `crates/glint-plugin` and has no GPUI or application service dependency. The main library re-exports the contract and owns `PluginPage`, the native renderer for validated declarative snapshots. Applications own plugin installation, enabled state, permission decisions, persistence, hook scheduling and their service implementations.

| Shared | Application-owned |
| --- | --- |
| Manifest, API version, contributions, pinned approvals | Supported hook names and contribution kinds |
| JSON host-call boundary, explicit grants | HTTP preparation, DB connections/queries |
| JS execution, generic crypto | KSO-1 and other business algorithms |
| Bounded page schema, native renderer/events | Where pages/commands appear, permission UI |

A database application registers `database.describe` under a metadata-read capability, invokes a page hook with a selected connection/table identifier, validates the resulting `Page` and mounts `PluginPage`. Subscribe to `PluginPageEvent`, route its action to a permitted hook and call `set_page` on the resulting snapshot. Plugins never receive connections, SQL executors, GPUI contexts or database credentials by default. Define separate capabilities for metadata, query, write and secret reads instead of a single unrestricted database permission.

The supplied `database_page` example produces a page with a refresh action and a native table using fixture metadata. V1 supports text, rows, columns, buttons and tables; editors, forms, dock registration, async hooks and hot reload are future extensions, not implemented APIs.

See `crates/glint-plugin/README.md` for contracts, execution limits and process supervision.

## Request actions

Glint API discovers `request.before-send` contributions from installed plugins. Each contribution supplies its own `id`, `title` and declared `hook`; one plugin can register multiple independently selectable actions. Request tabs display action titles and Switch controls, without plugin identities, versions or sources. Drag the left card handle to save request-local order. Only one card list is shown. Enabled actions execute in that order; disabling retains the saved binding. Existing requests pin their provider version and content; changed providers require explicit re-enabling. Plugin installation, versions, sources and permission declarations remain on the global plugin-management page.

```json
{"id":"sign-request","kind":"request.before-send","title":"WPS KSO-1 签名","hook":"request.before-send"}
```

Older manifests containing only `request.before-send` hooks are exposed as one legacy action. Older request approvals load as enabled action bindings. Installing a newer provider does not silently change already bound requests.
