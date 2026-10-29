# Glint Vault · Chrome / Edge / Ego Lite

Manifest V3 扩展，使用当前标签页的临时权限。默认展示全部账号，用户点击后将所选账号填入当前页面；默认需本机确认，可在密码箱设置免二次确认。不会自动提交登录，也不会扫描所有打开的标签页。

## 安装开发版

1. 打开 `chrome://extensions` 或 `edge://extensions`，启用开发者模式。
2. 选择“加载已解压的扩展程序”，指向本目录，复制扩展 ID。
3. 在仓库根目录运行对应的注册命令，把 `EXTENSION_ID` 替换成实际的 32 位 ID：

```sh
cargo run -p glint-vault --bin glint-vault-host -- --install chrome EXTENSION_ID
# Edge 单独注册，两个浏览器的扩展 ID 可能不同：
cargo run -p glint-vault --bin glint-vault-host -- --install edge EXTENSION_ID
# Ego Lite（macOS；当前版本使用 Chrome 的主机查找目录）：
cargo run -p glint-vault --bin glint-vault-host -- --install egolite EXTENSION_ID
```

注册器将主机可执行文件复制到 `Glint Vault/native-host/`，生成浏览器配置，Windows 额外注册当前用户 HKCU 键，不需要管理员权限。仅允许指定的精确扩展 ID。移动扩展目录可能改变开发版 ID，需要重新注册；生产发布后应使用商店分配的稳定 ID。

4. 启动 `cargo run -p glint-vault`，或打开 Glint API / Agent 的密码箱并解锁。
5. 为账号条目填写完整网站地址，如 `https://example.com/login`。在该网站点击扩展，选择账号，再切到本机应用确认填充。

也可点击账号旁的“复制密码”，在本机确认后手动粘贴。密码由本机应用直接写入系统剪贴板，不返回扩展；沿用密码箱的 30 秒清理和锁定清理机制（不会清除之后复制的其他内容）。此功能需要更新并重启 Glint Vault / API / Agent，再重新加载扩展。

密码箱顶部提供两个开关，设置保存在共享密码箱数据库中，Vault / API / Agent 共用，重启后保留：

- “浏览器操作免二次确认”（默认关闭）：开启后，解锁状态下扩展的填充和复制请求直接执行；关闭后恢复逐次确认。
- “按当前网址匹配账号”（默认关闭）：关闭时展示全部条目，也可选择未配置网址或属于其他网站的账号；开启后仅展示并允许使用当前网站匹配的条目。

两项设置独立生效，锁定限制、填充期间的页面跳转检查和剪贴板清理始终保留。账号列表分页读取全部条目，每页只返回名称和用户名。升级后重新注册本机主机（原安装命令及扩展 ID），以支持超过 50 条的分页列表，再重新加载扩展。

注册器默认读取 `GLINT_VAULT_DATA_DIR`；指定自定义目录时应与三个应用一致。安装后的主机从自身安装目录定位该数据库的浏览器连接，不依赖浏览器继承终端环境。软件升级后重新运行注册命令更新本机主机二进制。

## 行为和权限

- 权限只有 `activeTab`、`scripting`、`nativeMessaging`，没有 `<all_urls>`、持久 host permissions、网络服务或密码存储权限。
- `list` 仅返回账号的名称和用户名，不返回密码；每页最多 50 条，扩展自动读取后续页。是否按当前 origin 过滤取决于网址匹配设置。
- `fill` / `copy` 默认要求本机确认；60 秒内未确认则取消。开启“浏览器操作免二次确认”后直接执行。关闭扩展弹窗不取消后台的待确认请求。
- 浏览器填充请求会自动显示宿主的密码箱页。扩展连接最近一次解锁或打开密码箱页面的窗口；该窗口手动锁定/退出后，在其他已解锁窗口打开密码箱页面，或点击“让浏览器连接这个窗口”即可接管，无需再输入主密码。
- 在请求前后检查标签页 origin，并在实际填充函数内再次检查，页面跨 origin 跳转则取消。开启网址匹配时按 origin 精确匹配，不混用子域名、后缀相似域名、不同端口或 HTTP/HTTPS 的账号；关闭时由用户选择要填入当前页面的条目。
- 只填充顶层文档的可见登录表单，跳过隐藏和新密码/确认密码字段；暂不支持 iframe、Shadow DOM、自动保存/更新凭据、TOTP 或跨设备同步。
- 主密码只在本机 GPUI 页面输入。扩展不持久保存凭据，不把密码返回给扩展弹窗。凭据仅短暂经过本机连接、Native Messaging 和浏览器填充执行过程。

协议/控制器与表单填充有自动化测试；本目录是可加载的开发版，尚未发布到 Chrome Web Store / Edge Add-ons。已在 macOS Ego Lite 0.6.0.16 中注册本机主机，并从扩展页面验证 Native Messaging 到已解锁密码箱的请求成功；没有使用真实账号进行登录填充。Chrome / Edge 的实际安装和完整登录填充仍需按上述流程验收。

## 测试

```sh
node --test crates/glint-vault/browser-extension/tests/*.test.mjs
```

实现参考 Chrome 的 [Native Messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)、[activeTab](https://developer.chrome.com/docs/extensions/develop/concepts/activeTab) 和 Edge 的 [Native Messaging](https://learn.microsoft.com/en-us/microsoft-edge/extensions/developer-guide/native-messaging) 协议。
