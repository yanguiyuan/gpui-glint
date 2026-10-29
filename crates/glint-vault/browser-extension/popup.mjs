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
    status.textContent = result.entries.length
      ? (result.matchWebsite === false ? "全部账号 · 选择填充，或复制密码后手动粘贴" : "选择账号填充，或复制密码后手动粘贴")
      : (result.matchWebsite === false ? "密码箱中还没有账号，请先添加密码条目。" : "没有匹配的账号。可在密码箱关闭网址匹配，或为条目填写当前网站地址。");
    for (const entry of result.entries) {
      const card = document.createElement("div");
      card.className = "account";
      const label = document.createElement("span");
      const title = document.createElement("strong");
      const username = document.createElement("small");
      title.textContent = entry.title;
      username.textContent = entry.username || "无账号名";
      label.append(title, username);
      card.append(label);
      for (const [op, text] of [["fill", "填充"], ["copy", "复制密码"]]) {
        const button = document.createElement("button");
        button.textContent = text;
        card.append(button);
        button.addEventListener("click", async () => {
          const buttons = [...entries.querySelectorAll("button")];
          buttons.forEach((button) => {button.disabled = true;});
          status.textContent = `正在请求${text}；如需确认，请在本机密码箱处理。`;
          try {
            const reply = await chrome.runtime.sendMessage({op, entryId: entry.id, tabId: result.tabId, origin: result.origin});
            status.textContent = reply?.ok ? (op === "copy" ? "密码已复制，可返回网页手动粘贴。" : "已填充。请检查账号后自行登录。") : reply?.error ?? "操作失败";
          } catch {status.textContent = "连接中断，请解锁密码箱后重试";}
          finally {buttons.forEach((button) => {button.disabled = false;});}
        });
      }
      entries.append(card);
    }
  } catch (error) {
    site.textContent = "连接未完成";
    status.textContent = error?.message ?? "连接失败，请重试";
  }
}
document.querySelector("#refresh").addEventListener("click", load);
load();
