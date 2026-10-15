# Self-hosted plugin repositories

Implemented: a portable registry server, publishing CLI, reusable signed-source SDK and Glint API source/installation UI. No domain or cloud vendor is hard-coded. The SDK can be used directly or through `gpui_glint::plugin_registry` with the optional `plugin-registry` feature; this feature alone does not enable Boa. Its blocking HTTP/install methods belong on a background worker, not the UI or Tokio executor thread. The server validates source as data; it neither links Boa nor executes uploaded plugins.

| Package | Responsibility |
| --- | --- |
| `glint-plugin` | Manifest, runtime, approvals, generic page contract; Boa is an optional feature |
| `glint-plugin-registry` | Signed protocol, HTTPS client, pinned sources, rollback protection, versioned install store |
| `glint-plugin-server` | HTTP management API, SQLite metadata, immutable packages, persistent signing key |
| `glint-plugin-cli` / `glint-pluginctl` | Source configuration, publishing, versions, withdrawal/restore, install/uninstall |
| `glint-api` | Compatible-plugin filtering, request pre-actions, app-wide management and host permissions |

## Deploy on any server

Build native binaries:

```sh
cargo build --release -p glint-plugin-server -p glint-plugin-cli --locked
```

Create a private token file with at least 24 random characters, then run:

```sh
GLINT_REGISTRY_ADMIN_TOKEN_FILE=/secure/registry-token.txt \
  ./target/release/glint-plugin-server --data /srv/glint-registry --listen 127.0.0.1:8787
```

`GLINT_REGISTRY_ADMIN_TOKEN` is an alternative. Tokens are never CLI arguments or stored in client source configuration. The server prints its **public** key and source ID; `signing.key` is private and retained inside the data directory. Back up the complete data directory (SQLite, WAL and packages) consistently; preserve `signing.key` across restarts/upgrades. Resetting it changes source identity and requires client reconfiguration. Run one server process per data directory.

Docker Compose and an HTTPS reverse-proxy example are in `deployment/plugin-registry/`:

```sh
umask 077
printf 'GLINT_REGISTRY_ADMIN_TOKEN=%s\n' "$(openssl rand -hex 32)" > deployment/plugin-registry/registry.env
docker compose -f deployment/plugin-registry/compose.yml up -d --build
```

The container runs as a non-root user with a persistent named volume. It exposes port 8787 on loopback only. Configure Nginx or another TLS reverse proxy for your chosen domain; HTTPS is required outside loopback unless the operator explicitly opts into development HTTP. The supplied reverse-proxy config is a template, not a certificate provisioner. No SSH credentials are compiled into binaries.

## Configure a source and publish with CLI

```sh
cargo run -p glint-plugin-cli -- source inspect https://plugins.example.com
cargo run -p glint-plugin-cli -- source add company https://plugins.example.com \
  --public-key '<Base64 public key verified through a trusted channel>'
```

`inspect` reports the server's claimed identity, **not independent verification**. Compare it with the server's startup output or another trusted channel before adding. Source configuration pins this key. Adding a source does not execute plugins.

```sh
# Supply the management token through this process environment; do not put it in a command argument.
# GLINT_PLUGIN_TOKEN must already be set in the publishing environment.
cargo run -p glint-plugin-cli -- publish crates/glint-api/plugins/wps-kso1 \
  --source company --host glint-api@0.1.1 --changelog 'Initial KSO-1 support'
cargo run -p glint-plugin-cli -- list --source company
cargo run -p glint-plugin-cli -- yank com.wps.kso1 1.0.0 --source company
cargo run -p glint-plugin-cli -- restore com.wps.kso1 1.0.0 --source company
```

A version cannot be overwritten. Increment its manifest semver before publishing again. Withdrawal changes the signed catalog and prevents new installations; it does not remotely remove already installed/approved code. `--host` is repeatable for plugins supporting multiple applications. API compatibility, hooks, contributions and host minimum version all participate in client filtering.

CLI also supports `source list/remove`, `install`, `installed` and `uninstall`. `--home` / `GLINT_PLUGIN_HOME` selects its configuration/install directory, so the CLI can share an application's store. Its default is a separate `glint-pluginctl` data directory.

## Glint API client

Open the navigation rail → 插件管理 → 插件源. Enter a source alias, HTTPS base URL and verified public key. Fetch the catalog, review displayed capabilities and changelog, then install a selected version. Installation does not enable it; enable the installed version under the request’s 前置动作 tab, which supports ordering and disabling actions. Multiple sources and multiple retained versions are supported. Each request pins its selected provider version; installing a newer version does not silently change existing bindings. Removing a source preserves installed/approved versions.

Glint API stores registry data beside `workspace.sqlite3`, under `plugin-registry/`. To manage exactly those installations with CLI, pass this directory as `--home`. Source configurations, packages and cached indexes contain no publishing token. Old installed versions remain usable offline; new installation requires a fresh signed index.

## Protocol and integrity

- `GET /health`
- `GET /v1/info` — protocol, source ID, Ed25519 public key
- `GET /v1/index` — `{payload, signature}`, Base64 exact index bytes and detached signature
- `GET /v1/packages/{sha256}` — immutable `.glintpkg` package
- `POST /v1/publish` — publishing-token protected JSON `{package, hosts, changelog}`; `package` is Base64
- `POST /v1/plugins/{id}/{version}/yank` and `/restore` — token protected

The index includes a persisted sequence, issuance/expiry (24 hours), full manifests, host compatibility, sizes, hashes, changelogs and withdrawal state. Clients verify signatures against the configured key and source identity, reject rollback/expired metadata, verify package size/SHA-256, and cross-check its manifest. Management tokens use a constant-time digest comparison. The HTTP client refuses redirects and credential-bearing/query-bearing base URLs.

V1 `.glintpkg` is bounded JSON `{format:1, plugin:{manifest,source}}`, at most 512 KiB. Scripts are at most 256 KiB and manifests 64 KiB. No archive extraction, executable uploads, ES module dependencies or arbitrary asset trees exist in this format. Installation uses staging plus atomic directory rename and never replaces an existing immutable version. Packages are namespaced by signing-key identity and digest; request approvals bind source, plugin ID, version, source content and permissions. Same IDs from different sources cannot silently replace one another.

## Current boundaries

Reads are public; one management token owns all publishing/withdrawal operations. V1 has no web administration UI, per-publisher namespace roles, private-read tokens, key rotation workflow, plugin dependencies or automatic activation updates. Management is through the CLI/API. Source keys establish who published an artifact, not whether its code is trustworthy; application-specific capability checks and execution isolation remain required. No remote server deployment or certificate change is performed by creating these files.

## Deployed source

The verified `gateway` deployment is available at `https://gateway.yanguiyuan.cn/plugins`. Connection parameters and operational notes are in [the gateway deployment record](../deployment/plugin-registry/gateway/README.md).
