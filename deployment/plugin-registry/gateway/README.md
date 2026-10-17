# Gateway deployment

Deployed on 2026-10-15 to `gateway.yanguiyuan.cn`, Linux x86_64.

- Source alias: `gateway`
- Base URL: `https://gateway.yanguiyuan.cn/plugins`
- Ed25519 public key: `x1+OcqKIYuboSgnxFik6ApNpjOU4HeJiD6nMgQSYIeQ=`
- Source ID: `cbc98299e357699c4838f63564e22224f1ba8517d6a32396237e8786b08d1fcc`
- Published: `com.wps.kso1@1.0.0`, with the `sign-request` before-send action.

Enter the base URL and public key in Glint API → 插件管理 → 插件源. The key above was compared against the server's startup output through SSH before client registration.

```sh
glint-pluginctl source add gateway https://gateway.yanguiyuan.cn/plugins \
  --public-key 'x1+OcqKIYuboSgnxFik6ApNpjOU4HeJiD6nMgQSYIeQ='
glint-pluginctl list --source gateway
```

## Server operations

Deployment directory: `/srv/glint-plugin-registry`. The adjacent Compose/Caddy files record the actual configuration, and use the existing `api-gateway_internal` Docker network for HTTPS fallback to the existing gateway. The original gateway still owns port 80. Caddy owns 443 and automatically renews its certificate using TLS-ALPN validation; `/plugins/*` is forwarded with the prefix stripped. Registry port 8787 is bound to loopback.

```sh
cd /srv/glint-plugin-registry
sudo docker compose -p glint-plugin-registry ps
sudo docker compose -p glint-plugin-registry logs --tail 50 registry
sudo docker compose -p glint-plugin-registry restart registry
```

Both containers use `restart: unless-stopped`. Registry data and its private signing key persist in `data/`, owned by UID/GID 10001. Certificates persist in `caddy-data/`. The management token exists only in the private `registry.env` file on the server; obtain it through an authenticated server session and provide it to the publishing CLI as `GLINT_PLUGIN_TOKEN`. Never commit it or place its value in command arguments. This repository contains only public connection parameters.

`backups/initial-data.tar.gz` is the first consistent data backup, taken while the registry was stopped. Preserve the full data directory, including `signing.key`, when upgrading/restoring: replacing the signing key changes the source identity. Keep backups private.

The service image is `glint-plugin-registry:gateway`, runs as `10001:10001`, and contains the server and CLI. The uploaded source snapshot is in `source/`. `build-source/` contains the four service-related crates, a lockfile retaining their original dependency versions, and vendored dependencies for offline builds. No Boa runtime is linked into the server.

## Verified

HTTPS certificate validation; public health/info; server-key comparison; authenticated publish and immutable-version rejection; unauthenticated publish returns 401; pinned CLI signature verification and package installation; service stop/start preserves the public key, index sequence and package SHA-256; original HTTP gateway and HTTPS fallback return the same page.

## Public web directory (2026-10-17)

The edge-to-edge web catalogue is served at `https://gateway.yanguiyuan.cn/plugins/`; `/plugins` redirects to it. The page supports search, host filtering, version details, package downloads and copying source configuration. Its independent developer view (`#develop`) provides `glint-plugin-dev` Skill installation commands and the downloadable file at `https://gateway.yanguiyuan.cn/plugins/skills/glint-plugin-dev/SKILL.md`.

The registry includes support for declarative contribution `data` and inert OAuth preset manifests. Existing signing identity and published package bytes are preserved. `glint-plugin-registry:before-web` is the previous runtime image, and `backups/pre-web-data.tar.gz` is the consistent data snapshot taken during this upgrade. The directory remains public read-only; management still requires the private token.

The catalogue and developer guide are independent sidebar views. Branding reuses the Glint API software icon, including the favicon. The full-viewport shell has no clipping scroll ancestor, so its header and sidebar stay aligned during scrolling. The current service build context is `build-source/`; `source/` is the initial upload snapshot.

## Software distribution (2026-10-17)

The same deployment now serves the independent software download view at `https://gateway.yanguiyuan.cn/plugins/#software`, plus signed `/v1/apps/{app}/releases` feeds and immutable installer downloads. Configure the same base URL and trusted public key in Glint API 0.1.4's **软件更新 → 更新源设置** to replace GitHub for checks and downloads.

The upgrade preserved the plugin key, source ID, index sequence and package metadata. Before upgrade, a consistent backup was taken as `backups/pre-distribution-data.tar.gz`; the previous image is `glint-plugin-registry:before-distribution`. The runtime image ID after this upgrade is `sha256:6e0a31f2212d033cfb3695ce2c2712612910666382834d1d19653544f8060b1f`. `build-source/Dockerfile.distribution` and the offline vendored workspace reproduce the service build. Installer uploads have a dedicated larger Caddy body limit; JSON APIs remain bounded.

Use `glint-pluginctl software publish/list/yank/restore` with the private publishing token to manage software releases; see [the protocol and CLI guide](../../../docs/plugin-registry.md#software-distribution). Publish all platform installers together because existing release versions are immutable. GitHub releases and this distribution source are separate publishing destinations.
