import test from "node:test";
import assert from "node:assert/strict";
import {createController} from "../controller.mjs";
import {fillLoginForm} from "../fill.mjs";

function setup(native) {
  const calls = [];
  const tab = {id: 7, url: "https://example.com/login"};
  const api = {tabs: {query: async () => [tab], get: async () => tab},
    scripting: {executeScript: async (args) => {calls.push(args); return [{result: {filled: true}}];}}};
  return {handle: createController(api, native), tab, calls, api};
}
test("popup receives metadata only and fill only returns status", async () => {
  const reply = {ok: true, origin: "https://example.com", username: "alice", password: "secret"};
  const {handle, calls} = setup(async ({op}) => op === "list" ? {ok: true, entries: [{id: "one", title: "Example", username: "alice", password: "must-not-reach-popup"}]} : reply);
  const list = await handle({op: "list"});
  assert.deepEqual(list.entries, [{id: "one", title: "Example", username: "alice"}]);
  const filled = await handle({op: "fill", entryId: "one", tabId: list.tabId, origin: list.origin});
  assert.deepEqual(filled, {ok: true, filled: true});
  assert.deepEqual(calls[0].target, {tabId: 7});
  assert.deepEqual(calls[0].args, ["https://example.com", "alice", "secret"]);
  assert.equal(reply.password, "");
});

test("all-account listing collects every page and strips passwords", async () => {
  const offsets = [];
  const {handle} = setup(async request => {
    const offset = request.offset ?? 0;
    offsets.push(offset);
    return {ok:true, match_website:false, next_offset:offset === 0 ? 50 : null,
      entries:Array.from({length:offset === 0 ? 50 : 2}, (_,index)=>({id:String(offset+index),title:"test",username:"user",password:"secret"}))};
  });
  const result = await handle({op:"list"});
  assert.deepEqual(offsets, [0,50]);
  assert.equal(result.entries.length,52);
  assert.equal(result.matchWebsite,false);
  assert(!JSON.stringify(result).includes("secret"));
});
test("cross-origin navigation during approval cancels injection", async () => {
  const {handle, tab, calls} = setup(async () => {
    tab.url = "https://example.com.evil.test/";
    return {ok: true, origin: "https://example.com", username: "alice", password: "secret"};
  });
  const result = await handle({op: "fill", entryId: "one", tabId: 7, origin: "https://example.com"});
  assert.equal(result.ok, false);
  assert.equal(calls.length, 0);
});
test("denial and mismatched native origin never inject credentials", async () => {
  for (const reply of [{ok: false, error: "已取消填充"}, {ok: true, origin: "https://evil.test", username: "x", password: "secret"}]) {
    const {handle, calls} = setup(async () => reply);
    assert.equal((await handle({op: "fill", entryId: "one", tabId: 7, origin: "https://example.com"})).ok, false);
    assert.equal(calls.length, 0);
  }
});
test("restricted pages are refused before native messaging", async () => {
  let contacted = false;
  const {handle, tab} = setup(async () => {contacted = true;});
  tab.url = "chrome://extensions/";
  assert.equal((await handle({op: "list"})).ok, false);
  assert.equal(contacted, false);
});
test("fill function refuses wrong document origin before touching the DOM", () => {
  globalThis.location = {origin: "https://elsewhere.test"};
  assert.match(fillLoginForm("https://example.com", "alice", "secret").error, /页面已跳转/);
  delete globalThis.location;
});
test("fills a visible login form with native input/change events and never submits", () => {
  class Input {
    constructor(type, name, autocomplete, visible = true) {
      Object.assign(this, {type, name, id: name, autocomplete, isConnected: true, disabled: false, readOnly: false, visible, events: []});
    }
    get value() {return this._value ?? "";}
    set value(value) {this._value = value;}
    getClientRects() {return this.visible ? [{}] : [];}
    compareDocumentPosition() {return 4;}
    dispatchEvent(event) {this.events.push(event.type);}
  }
  const username = new Input("email", "email", "username");
  const hidden = new Input("password", "trap", "", false);
  const signup = new Input("password", "new_password", "new-password");
  const password = new Input("password", "password", "current-password");
  const form = {querySelectorAll: () => [username], submit: () => {throw new Error("must not submit");}};
  password.form = form;
  Object.assign(globalThis, {
    HTMLInputElement: Input, Node: {DOCUMENT_POSITION_FOLLOWING: 4},
    location: {origin: "https://example.com"},
    getComputedStyle: () => ({visibility: "visible", display: "block"}),
    document: {activeElement: null, querySelectorAll: () => [hidden, signup, password]},
  });
  try {
    assert.deepEqual(fillLoginForm("https://example.com", "alice", "secret"), {filled: true});
    assert.equal(username.value, "alice");
    assert.equal(password.value, "secret");
    assert.equal(hidden.value, "");
    assert.equal(signup.value, "");
    assert.deepEqual(password.events, ["input", "change"]);
  } finally {
    for (const key of ["HTMLInputElement", "Node", "location", "getComputedStyle", "document"]) delete globalThis[key];
  }
});

test("missing URL metadata falls back to the active main-frame address", async () => {
  let requested;
  const {handle, tab, api} = setup(async (request) => {requested = request; return {ok: true, entries: []};});
  delete tab.url;
  api.scripting.executeScript = async ({target}) => {
    assert.deepEqual(target, {tabId: 7});
    return [{result: "https://example.com/login"}];
  };
  const result = await handle({op: "list"});
  assert.equal(result.ok, true);
  assert.equal(requested.origin, "https://example.com");
});

test("login forms mislabelled new-password fill, but signup/reset forms remain excluded", () => {
  class Input {
    constructor(type, name) {
      Object.assign(this, {type, name, id: name, autocomplete: "new-password", isConnected: true});
    }
    get value() {return this._value ?? "";}
    set value(value) {this._value = value;}
    getClientRects() {return [{}];}
    dispatchEvent() {}
    compareDocumentPosition() {return 4;}
  }
  for (const [action, passwordCount, expected, roleButton = false] of [
    ["立即登录", 1, true], ["Sign in", 1, true], ["注册", 1, false],
    ["重置密码", 1, false], ["Sign up", 1, false], ["登录", 2, false], ["", 1, false],
    ["立即登录", 1, true, true], ["注册", 1, false, true],
  ]) {
    const account = new Input("text", "account");
    const passwords = Array.from({length: passwordCount}, (_, i) => new Input("password", i ? "confirm" : "password"));
    // Login actions may stay disabled until both required fields have values.
    const button = {textContent: action, disabled: true, getClientRects: () => [{}]};
    const form = {querySelectorAll: selector => selector.startsWith("button")
      ? ((!roleButton || selector.includes('[role="button"]')) ? [button] : []) : [account]};
    for (const input of [account, ...passwords]) input.form = form;
    Object.assign(globalThis, {
      HTMLInputElement: Input, Node: {DOCUMENT_POSITION_FOLLOWING: 4},
      location: {origin: "https://example.com"},
      getComputedStyle: () => ({visibility: "visible", display: "block"}),
      document: {activeElement: null, querySelectorAll: () => passwords},
    });
    try {
      const result = fillLoginForm("https://example.com", "test-user", "test-password");
      assert.equal(result.filled, expected, `${action}, ${passwordCount} passwords`);
      assert.equal(passwords[0].value, expected ? "test-password" : "");
      assert.equal(account.value, expected ? "test-user" : "");
    } finally {
      for (const key of ["HTMLInputElement", "Node", "location", "getComputedStyle", "document"]) delete globalThis[key];
    }
  }
});

test("copy delegates to the approved native clipboard without injection or secrets in popup", async () => {
  const requests = [];
  const {handle, calls} = setup(async request => {
    requests.push(request);
    return {ok:true, copied:true};
  });
  assert.deepEqual(await handle({op:"copy", entryId:"one", tabId:7, origin:"https://example.com"}), {ok:true, copied:true});
  assert.deepEqual(requests, [{op:"copy", origin:"https://example.com", entry_id:"one"}]);
  assert.equal(calls.length, 0);
});

test("copy rejects denial, old native hosts, and changed origins", async () => {
  for (const reply of [{ok:false,error:"已取消操作"}, {ok:true}]) {
    const {handle} = setup(async () => reply);
    assert.equal((await handle({op:"copy", entryId:"one", tabId:7, origin:"https://example.com"})).ok, false);
  }
  let contacted = false;
  const {handle} = setup(async () => {contacted=true;});
  assert.equal((await handle({op:"copy", entryId:"one", tabId:7, origin:"https://other.test"})).ok, false);
  assert.equal(contacted, false);
});

test("injected form errors are shown in the popup without exposing credentials", async () => {
  const reply = {ok: true, origin: "https://example.com", username: "alice", password: "secret"};
  const {handle, api} = setup(async () => reply);
  api.scripting.executeScript = async () => [{result: {filled: false, error: "未找到可见的登录密码框"}}];
  assert.deepEqual(await handle({op:"fill", entryId:"one", tabId:7, origin:"https://example.com"}),
    {ok: false, error: "未找到可见的登录密码框"});
  assert.equal(reply.password, "");
});
test("unavailable or malformed tab URLs produce an actionable message instead of URL exceptions", async () => {
  for (const address of [undefined, "", "not-a-url", "chrome://newtab/"]) {
    let contacted = false;
    const {handle, tab, api} = setup(async () => {contacted = true;});
    tab.url = address;
    api.scripting.executeScript = async () => {throw new Error("Cannot access chrome:// URL");};
    const result = await handle({op: "list"});
    assert.equal(result.ok, false);
    assert.match(result.error, /普通网站/);
    assert.doesNotMatch(result.error, /Failed to construct|Invalid URL/);
    assert.equal(contacted, false);
  }
});
