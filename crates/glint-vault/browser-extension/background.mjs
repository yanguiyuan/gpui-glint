import {createController} from "./controller.mjs";
function native(message) {
  return new Promise((resolve, reject) => {
    const port = chrome.runtime.connectNative("com.glint.vault");
    let finished = false;
    const finish = (error, response) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      port.disconnect();
      error ? reject(error) : resolve(response);
    };
    const timer = setTimeout(() => finish(new Error("本机确认超时，请重试")), 65000);
    port.onMessage.addListener((response) => finish(null, response));
    port.onDisconnect.addListener(() => {
      const detail = chrome.runtime.lastError?.message;
      finish(new Error(detail ? `本机连接失败：${detail}。请检查本机主机是否已注册。` : "本机连接已断开，请解锁密码箱后重试"));
    });
    port.postMessage(message);
  });
}
const handle = createController(chrome, native);
chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (sender.id !== chrome.runtime.id || sender.url !== chrome.runtime.getURL("popup.html")) return false;
  handle(message).then(respond);
  return true;
});
