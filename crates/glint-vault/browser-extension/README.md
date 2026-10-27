# Glint Vault · Chrome / Edge / Ego Lite

Manifest V3 扩展，使用当前标签页的临时权限。用户点击账号后，在本机密码箱中确认，扩展才会将所选账号填入**相同 origin** 的可见登录表单。不会自动提交登录，也不会扫描所有打开的标签页。

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

注册器默认读取 `GLINT_VAULT_DATA_DIR`；指定自定义目录时应与三个应用一致。安装后的主机从自身安装目录定位该数据库的浏览器连接，不依赖浏览器继承终端环境。软件升级后重新运行注册命令更新本机主机二进制。

## 行为和权限

- 权限只有 `activeTab`、`scripting`、`nativeMessaging`，没有 `<all_urls>`、持久 host permissions、网络服务或密码存储权限。
- `list` 仅返回当前 origin 下最多 50 个账号的名称和用户名，不返回密码。
- `fill` 每次均要求本机确认；60 秒内未确认则取消。关闭扩展弹窗不取消后台的待确认填充。
- 浏览器填充请求会自动显示宿主的密码箱页。扩展连接最近一次解锁或打开密码箱页面的窗口；该窗口手动锁定/退出后，在其他已解锁窗口打开密码箱页面，或点击“让浏览器连接这个窗口”即可接管，无需再输入主密码。
- 在确认前后检查标签页 origin，并在实际填充函数内再次检查；不会把 `example.com` 的凭据填到子域名、后缀相似域名、不同端口或 HTTP/HTTPS 的另一方。
- 只填充顶层文档的可见登录表单，跳过隐藏和新密码/确认密码字段；暂不支持 iframe、Shadow DOM、自动保存/更新凭据、TOTP 或跨设备同步。
- 主密码只在本机 GPUI 页面输入。扩展不持久保存凭据，不把密码返回给扩展弹窗。凭据仅短暂经过本机连接、Native Messaging 和浏览器填充执行过程。

协议/控制器与表单填充有自动化测试；本目录是可加载的开发版，尚未发布到 Chrome Web Store / Edge Add-ons。已在 macOS Ego Lite 0.6.0.16 中注册本机主机，并从扩展页面验证 Native Messaging 到已解锁密码箱的请求成功；没有使用真实账号进行登录填充。Chrome / Edge 的实际安装和完整登录填充仍需按上述流程验收。

## 测试

```sh
node --test crates/glint-vault/browser-extension/tests/*.test.mjs
```

实现参考 Chrome 的 [Native Messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)、[activeTab](https://developer.chrome.com/docs/extensions/develop/concepts/activeTab) 和 Edge 的 [Native Messaging](https://learn.microsoft.com/en-us/microsoft-edge/extensions/developer-guide/native-messaging) 协议。
