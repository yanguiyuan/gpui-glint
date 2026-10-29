// This function is self-contained because Chrome serializes it into the tab.
export function fillLoginForm(expectedOrigin, username, password) {
  try {
    if (location.origin !== expectedOrigin) throw new Error("页面已跳转，请重新打开扩展");
    const rendered = (element) =>
      element.getClientRects().length > 0 &&
      getComputedStyle(element).visibility !== "hidden" &&
      getComputedStyle(element).display !== "none";
    const visible = (element) => !element.disabled && !element.readOnly && rendered(element);
    const passwordInputs = [...document.querySelectorAll('input[type="password"]')].filter(visible);
    const loginForm = (element) => {
      const form = element.form;
      if (!form || passwordInputs.filter(input => input.form === form).length !== 1) return false;
      // Some login pages use new-password on every input to suppress browser
      // autofill. Accept that only with a clear login action in the same form.
      const actions = [...form.querySelectorAll('button,[role="button"],input[type="submit"],input[type="button"]')]
        .filter(rendered).map(button => button.textContent || button.value || "").join(" ");
      return /登录|登入|登錄|\b(?:log\s*in|sign\s*in)\b/i.test(actions) &&
        !/注册|註冊|重置|修改密码|设置密码|設定密碼|\b(?:register|sign\s*up|reset|change\s*password)\b/i.test(actions);
    };
    const passwords = passwordInputs.filter((element) =>
      !/(confirm|confirmation|repeat|new.?password)/i.test(`${element.name} ${element.id}`) &&
      (element.autocomplete !== "new-password" || loginForm(element)));
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
  } catch (error) {
    // Chrome may omit the result of a throwing injected function. Return an
    // explicit status so the popup can explain why the form was rejected.
    return {filled: false, error: error?.message ?? "未能填充登录表单"};
  }
}
