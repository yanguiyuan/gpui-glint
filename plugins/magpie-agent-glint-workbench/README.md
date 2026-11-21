# magpie-agent-glint-workbench

在 Magpie 里为 **Glint Workbench** 选一个模型，把 Magpie 网关合并成 Glint Workbench 的供应商。

## 用法

```sh
magpie plugin add ./plugins/magpie-agent-glint-workbench
```

然后在 Magpie 的 **Agent** 页找到 `Glint Workbench` 这一行，选一个模型即可。之后可以随时在这一行换模型，Glint Workbench 会自动同步。

断开（选回它自己的模型，或关掉这一行的开关）时，Magpie 会把这个文件恢复成原来的样子，Glint Workbench 里对应的供应商也随之移除。

## 写入什么

Magpie 把供应商写进 Glint Workbench 自己的配置文件 `~/.glint-workbench/providers.json`：

```json
{
  "model": "our-free-model/trae/deepseek-v4.1-flash",
  "providers": [
    {
      "id": "magpie",
      "name": "Magpie",
      "base_url": "http://127.0.0.1:3425/v1",
      "api_key": "…",
      "models_url": "http://127.0.0.1:3425/v1/models",
      "models": ["…"]
    }
  ]
}
```

Glint Workbench 每 2 秒读一次这个文件，把里面的供应商和它自己的供应商库合并：同 id 覆盖、文件里删掉的移除，用户自己添加的供应商不受影响。`api_key` 会被搬进 Glint Workbench 的本地密钥库，不留在供应商元数据里。

## 说明

- Magpie 只在配置文件已经存在时才写入，所以 Glint Workbench 启动时会先把这个文件建好（`{"providers": []}`）。
- 顶层 `model` 是兜底：给没有自己 `model` 的供应商补上选中的模型。
- 网关地址默认是 `http://127.0.0.1:3425/v1`，Magpie 换了端口时从 `~/.config/magpie/gateway.json` 读取。
