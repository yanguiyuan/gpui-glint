const site = document.querySelector("#site");
const status = document.querySelector("#status");
const entries = document.querySelector("#entries");
async function load() {
  entries.replaceChildren();
  status.textContent = "正在连接本机密码箱…";
  try {
    const result = await chrome.runtime.sendMessage({op: "list"});
    if (!result?.ok) throw new Error(result?.error ?? "无法连接密码箱");
    site.textContent = result.origin;
    status.textContent = result.entries.length ? "选择要填充的账号" : "没有匹配的账号。请在密码箱中为条目填写这个网站的完整地址。";
    for (const entry of result.entries) {
      const button = document.createElement("button");
      button.className = "account";
      const label = document.createElement("span");
      const title = document.createElement("strong");
      const username = document.createElement("small");
      const action = document.createElement("em");
      title.textContent = entry.title;
      username.textContent = entry.username || "无账号名";
      action.textContent = "填充";
      label.append(title, username);
      button.append(label, action);
      button.addEventListener("click", async () => {
        const buttons = [...entries.querySelectorAll("button")];
        buttons.forEach((button) => {button.disabled = true;});
        status.textContent = "请在本机密码箱确认本次填充；关闭弹窗后仍会继续等待。";
        try {
          const reply = await chrome.runtime.sendMessage({op: "fill", entryId: entry.id, tabId: result.tabId, origin: result.origin});
          status.textContent = reply?.ok ? "已填充。请检查账号后自行登录。" : reply?.error ?? "填充失败";
        } catch {status.textContent = "连接中断，请解锁密码箱后重试";}
        finally {buttons.forEach((button) => {button.disabled = false;});}
      });
      entries.append(button);
    }
  } catch (error) {
    site.textContent = "连接未完成";
    status.textContent = error?.message ?? "连接失败，请重试";
  }
}
document.querySelector("#refresh").addEventListener("click", load);
load();
