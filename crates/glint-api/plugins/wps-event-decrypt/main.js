// WPS event security: verify HMAC first, then AES-256-CBC with hex(MD5(SK)).
glint.on("websocket.message.decode", (event) => {
  if (!event || typeof event.encrypted_data !== "string")
    return { error: "当前事件没有 encrypted_data 加密字段" };
  if (typeof event.topic !== "string" || typeof event.nonce !== "string" ||
      typeof event.signature !== "string" || !Number.isSafeInteger(event.time))
    return { error: "事件格式无效：缺少 topic、nonce、time 或 signature" };
  const appId = glint.host.call("environment.get", "wps_app_id");
  const appKey = glint.host.call("environment.get", "wps_app_key");
  const content = [appId, event.topic, event.nonce, event.time, event.encrypted_data].join(":");
  if (!glint.host.call("crypto.hmacSha256VerifyBase64Url", appKey, content, event.signature))
    return { error: "事件签名校验失败，请检查 wps_app_id 和 wps_app_key" };
  let data;
  try {
    const key = glint.host.call("crypto.md5Hex", appKey);
    const text = glint.host.call("crypto.aes256CbcPkcs7DecryptBase64", key, event.nonce, event.encrypted_data);
    data = JSON.parse(text);
  } catch (_) {
    return { error: "事件解密失败，请检查密文、nonce、应用密钥及 JSON 格式" };
  }
  const detail = data && typeof data === "object" ? data : {};
  const parts = [detail.event_code || event.topic, event.operation];
  const label = detail.title || detail.name || detail.chat_id || detail.file_id || detail.id;
  if (typeof label === "string" || typeof label === "number") parts.push(String(label));
  return { title: parts.filter(Boolean).join(" · "), data: { topic: event.topic, operation: event.operation, time: event.time, nonce: event.nonce, signature_verified: true, data } };
});
