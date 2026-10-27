# Glint Vault

本地加密密码箱，提供独立桌面程序、可嵌入 Glint API / Agent 的原生页面，以及 Chrome / Edge 浏览器扩展（兼容 macOS Ego Lite）。

```sh
cargo run -p glint-vault
```

支持创建密码箱、主密码解锁、账号条目的增删改查、搜索、密码显隐与复制、生成 24 位随机密码。主密码至少 12 个字符，遗忘后无法恢复。复制的密码在 30 秒后清除，期间若用户复制了其他内容则不覆盖。条目删除需要确认。

## 一个密码箱，三个入口

独立程序和 API / Agent 侧栏的钥匙图标默认使用**同一个** `Glint Vault/vault.sqlite3`：

| 平台 | 默认路径 |
| --- | --- |
| macOS | `~/Library/Application Support/Glint Vault/vault.sqlite3` |
| Windows | `%APPDATA%/Glint Vault/vault.sqlite3` |
| Linux | `$XDG_DATA_HOME/Glint Vault/vault.sqlite3`，默认 `~/.local/share/Glint Vault/vault.sqlite3` |

可通过 `GLINT_VAULT_DATA_DIR` 统一覆盖目录。它独立于 `GLINT_API_DATA_DIR` 和 `GLINT_AGENT_DATA_DIR`。各进程独立解锁，主密码和密钥不会在应用之间传递。**同一次软件运行中，切换页面保留解锁状态**；只有手动锁定或退出才清除会话密钥，下一次启动需重新解锁。切换页面会重新遮罩已显示的密码，保留未保存的编辑。

未编辑时，每 2 秒检查一次数据库版本并刷新其他窗口的修改。编辑中保留草稿；保存使用数据库 revision 条件更新，检测到其他窗口已提交时拒绝覆盖并提示处理。密码箱不写入请求历史、会话、记忆或供应商配置，Agent 不会自动获得读取密码箱的工具。

## 集成

`glint-vault` 是独立 workspace crate，包含存储模块 `store` 和可嵌入的 `VaultPage`；`src/main.rs` 仅负责平台启动与应用外框。宿主保留自己的导航和服务，不需要新增全局插件框架或让 UI 库依赖密码箱。

```toml
[dependencies]
glint-vault = { path = "../glint-vault" }
```

```rust,ignore
let vault = cx.new(|cx| {
    glint_vault::VaultPage::new(glint_vault::default_path(), theme, cx)
});
// 导航/主题变化时更新，隐藏页面不会锁定。
vault.update(cx, |vault, cx| vault.set_host_state(is_visible, theme, cx));
// 把 vault.clone() 作为 AppLayout 的 content，宿主控制侧栏钥匙图标。
// 收到 VaultEvent::BrowserFillRequested 时显示密码箱页面，供用户确认。
```

Glint API 和 Agent 已接入这些接口。浏览器填充请求会自动切到密码箱页面；事件中不包含凭据。

## 存储边界

使用 RustCrypto 的 Argon2id v19（64 MiB、3 次迭代、1 lane）从主密码和独立随机盐派生 256 位密钥，以 AES-256-GCM 加密整个条目集合；每次保存生成新的 96 位随机 nonce，格式、盐和 revision 参与认证。名称、账号、网站、密码和备注都只以密文落盘。SQLite 只存格式版本、revision、盐、nonce 和密文，没有可搜索的明文索引；搜索在已解锁的内存数据中完成。

密钥、临时明文缓冲区和存储条目在释放时尽力清零。锁定清空编辑器和当前复制的密码。共享 TextEntry 的 `masked(true)` 遮罩渲染和原生文本查询，禁止普通复制/剪切；“复制密码”是独立的显式操作。通用 GPUI 字符串、系统剪贴板、输入法和浏览器进程可能保留内存副本，这不是对进程内存完全擦除的保证。未做独立安全审计，首版没有云同步、自动备份、主密码恢复或浏览器自动保存密码功能。

## Chrome / Edge / Ego Lite 扩展

见 [browser-extension/README.md](browser-extension/README.md)。扩展不保存主密码、数据库副本或账号密码，不连接云服务。使用 Native Messaging 与本机主机通信；主机仅转发到最近解锁窗口的认证 loopback 连接，本身不解密密码箱。

## 验证

```sh
cargo test -p glint-vault --lib
cargo test -p gpui-glint --lib
node --test crates/glint-vault/browser-extension/tests/*.test.mjs
cargo check -p glint-vault -p glint-api -p glint-agent
```
