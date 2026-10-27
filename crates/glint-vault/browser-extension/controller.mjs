import {fillLoginForm} from "./fill.mjs";
export function createController(api, native) {
  const filling = new Set();
  const unavailable = () => new Error("无法读取当前网页地址。请切到普通网站的登录页，再点击扩展图标重试；浏览器设置页和新标签页不支持填充。");
  const website = async (tab) => {
    if (!tab || !Number.isInteger(tab.id)) throw unavailable();
    let address = tab.url;
    if (typeof address !== "string" || !address.trim()) {
      // activeTab can permit injection even when tabs.query omits URL metadata.
      // Read only the current main-frame address; never ask for persistent tabs permission.
      try {
        const frames = await api.scripting.executeScript({target: {tabId: tab.id}, func: () => location.href});
        address = frames[0]?.result;
      } catch { throw unavailable(); }
    }
    let url;
    try {
      if (typeof address !== "string" || !address.trim()) throw unavailable();
      url = new URL(address);
    } catch { throw unavailable(); }
    if (!["https:", "http:"].includes(url.protocol) || !url.hostname || url.username || url.password)
      throw unavailable();
    return url.origin;
  };
  return async function handle(message) {
    try {
      if (message.op === "list") {
        const [tab] = await api.tabs.query({active: true, lastFocusedWindow: true});
        if (!tab || !Number.isInteger(tab.id)) throw new Error("无法读取当前标签页");
        const origin = await website(tab);
        const reply = await native({op: "list", origin, entry_id: null});
        if (!reply?.ok) throw new Error(reply?.error ?? "密码箱未连接");
        if (!Array.isArray(reply.entries)) throw new Error("密码箱回复格式无效");
        // Send only account metadata to the popup, never secrets.
        const entries = reply.entries.slice(0, 50).map(({id, title, username}) => ({id, title, username}));
        return {ok: true, origin, tabId: tab.id, entries};
      }
      if (message.op !== "fill" || !Number.isInteger(message.tabId) ||
          typeof message.entryId !== "string" || message.entryId.length > 64)
        throw new Error("填充请求无效");
      if (filling.has(message.tabId)) throw new Error("正在等待本机确认");
      const origin = await website(await api.tabs.get(message.tabId));
      if (origin !== message.origin) throw new Error("页面已跳转，请重新打开扩展");
      filling.add(message.tabId);
      let reply;
      try {
        reply = await native({op: "fill", origin, entry_id: message.entryId});
        if (!reply?.ok) throw new Error(reply?.error ?? "未获准填充");
        if (reply.origin !== origin || typeof reply.username !== "string" ||
            typeof reply.password !== "string" || reply.password.length > 8192)
          throw new Error("密码箱回复格式无效");
        if (await website(await api.tabs.get(message.tabId)) !== origin)
          throw new Error("页面已跳转，已取消填充");
        const results = await api.scripting.executeScript({
          target: {tabId: message.tabId},
          func: fillLoginForm,
          args: [origin, reply.username, reply.password],
        });
        if (!results[0]?.result?.filled) throw new Error("未能填充登录表单");
        return {ok: true, filled: true};
      } finally {
        if (reply) { reply.password = ""; reply.username = ""; }
        filling.delete(message.tabId);
      }
    } catch (error) { return {ok: false, error: error?.message ?? "操作失败，请重试"}; }
  };
}
