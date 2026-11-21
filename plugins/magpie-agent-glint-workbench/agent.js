// Magpie 把供应商写进 glint 自己的配置文件 ~/.glint-workbench/providers.json。
// Glint Workbench 读这个文件，把里面的供应商和数据库里的供应商合并。
export const agent = {
  id: "glint-workbench",
  name: "Glint Workbench",
  config: "~/.glint-workbench/providers.json",
  format: "json",
  model: "model",
  notice: "Glint Workbench 会在 2 秒内合并这个供应商，模型可以随时在 Magpie 里换。",
};

export function connect({ gateway, model, models }) {
  const base = gateway?.v1 ?? "http://127.0.0.1:3425/v1";
  const list = (models ?? []).filter((entry) => entry?.id);
  return {
    providers: [
      {
        id: "magpie",
        name: "Magpie",
        base_url: base,
        api_key: gateway?.key ?? "magpie",
        models_url: `${base}/models`,
        model: model?.id ?? "",
        models: list.map((entry) => entry.id),
        model_context_windows: Object.fromEntries(
          list.filter((entry) => entry.context).map((entry) => [entry.id, entry.context]),
        ),
        context_window_overrides: Object.fromEntries(
          list.filter((entry) => entry.output).map((entry) => [entry.id, entry.output]),
        ),
      },
    ],
  };
}
