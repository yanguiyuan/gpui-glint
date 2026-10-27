// This function is self-contained because Chrome serializes it into the tab.
export function fillLoginForm(expectedOrigin, username, password) {
  if (location.origin !== expectedOrigin) throw new Error("页面已跳转，请重新打开扩展");
  const visible = (element) => !element.disabled && !element.readOnly &&
    element.getClientRects().length > 0 &&
    getComputedStyle(element).visibility !== "hidden" &&
    getComputedStyle(element).display !== "none";
  const passwords = [...document.querySelectorAll('input[type="password"]')]
    .filter((element) => visible(element) && element.autocomplete !== "new-password" &&
      !/(confirm|confirmation|repeat|new.?password)/i.test(`${element.name} ${element.id}`));
  const target = passwords.find((input) => input === document.activeElement) ??
    passwords.find((input) => input.autocomplete === "current-password") ?? passwords[0];
  if (!target) throw new Error("未找到可见的登录密码框；暂不填充内嵌网页或注册表单");
  const root = target.form ?? document;
  const candidates = [...root.querySelectorAll('input[type="text"],input[type="email"],input[type="tel"],input:not([type])')]
    .filter(visible);
  const account = candidates.find((input) => input.autocomplete === "username") ??
    candidates.find((input) => input.type === "email" || /user|email|login|account/i.test(`${input.name} ${input.id}`)) ??
    candidates.filter((input) => input.compareDocumentPosition(target) & Node.DOCUMENT_POSITION_FOLLOWING).at(-1);
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  const update = (input, value) => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", {bubbles: true}));
    input.dispatchEvent(new Event("change", {bubbles: true}));
  };
  if (account && username) update(account, username);
  if (location.origin !== expectedOrigin || !target.isConnected) throw new Error("页面已变化，请重试");
  update(target, password);
  return {filled: true};
}
