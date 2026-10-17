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

## OAuth presets

Glint API discovers inert `oauth.preset` contributions alongside request actions. The native OAuth editor lists the latest installed version in each plugin family under **授权预设**, as compact, single-line contribution titles. Plugin names and versions remain in plugin management. Selecting one copies its settings into a new connection with an empty Client ID; users enter their own credentials and explicitly start authorization. Existing request configurations do not depend on the plugin after copying. Removing a plugin removes its menu entries; refreshing or updating it does not modify copied settings.

The bundled `crates/glint-api/plugins/wps-oauth` is independent of `wps-kso1`. It has no hooks or capabilities and supplies this manifest:

```json
{
  "id": "com.wps.oauth",
  "name": "WPS 用户授权",
  "version": "1.0.0",
  "api_version": 1,
  "entry": "main.js",
  "capabilities": [],
  "hooks": [],
  "contributions": [{
    "id": "user-authorization",
    "kind": "oauth.preset",
    "title": "WPS 用户授权",
    "data": {
      "authorization_url": "https://openapi.wps.cn/oauth2/auth",
      "token_url": "https://openapi.wps.cn/oauth2/token",
      "scope": "kso.user_base.read",
      "pkce": false
    }
  }]
}
```

Keep an inert `main.js` (an empty file or comment) for the existing plugin-package format. Neither discovery nor selection executes it. Preset contributions omit `hook` and use validated `data`; existing hook contributions retain their serialization and approval fingerprints. The reusable contract accepts generic JSON data; interpretation and OAuth policy belong to Glint API.

`authorization_url` and `token_url` are required. Optional fields are `scope` (empty by default), `pkce` (true by default), `basic_auth` (false), `redirect_uri` (host loopback default), `manual_callback` (false), and `rolling_refresh_expiry` (false). The host accepts `refresh_expires_in` or `refresh_token_expires_in` from Token responses. By default, refresh cannot extend the original deadline; `rolling_refresh_expiry: true` uses the expiry reported with each newly rotated token, retaining the old deadline if the response omits expiry. This policy is copied and persisted with the connection. Endpoints require HTTPS, with HTTP permitted only for 127.0.0.1 development. Callback restrictions match the native OAuth editor. Client IDs, secrets, access tokens, unknown fields, and hook-driven presets are rejected. A plugin providing only presets must declare no hooks or capabilities; a plugin combining presets and request actions still needs the action permissions and separate request-level approval. The host retains the browser, state/PKCE verification, Token exchange/refresh and system credential store.

Custom presets use the same local plugin directory, refresh operation, signed registry packages and installation flow as request actions. Publishing a preset never embeds developer credentials.

The independent bundled `crates/glint-api/plugins/feishu-oauth` preset uses Feishu's authorization endpoint `https://accounts.feishu.cn/open-apis/authen/v1/authorize` and v3 Token endpoint `https://accounts.feishu.cn/oauth/v3/token`, body client credentials, PKCE off, space-separated scopes, and `rolling_refresh_expiry: true`. Its default `offline_access` scope requires enabling the corresponding permission in the Feishu app; add the user permissions needed for your APIs and register the callback URL. See Feishu's [authorization code](https://open.feishu.cn/document/authentication-management/access-token/obtain-oauth-code) and [v3 Token](https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/authentication-management/access-token/get-user-access-token-v3) documentation.
