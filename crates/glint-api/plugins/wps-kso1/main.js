// glint-api owns the request contract; the shared plugin runtime knows no HTTP algorithms.
glint.on("request.before-send", (request) => {
  const appId = glint.host.call("environment.get", "wps_app_id");
  const appKey = glint.host.call("environment.get", "wps_app_key");
  if (!appId || !appKey) throw new Error("Missing WPS credentials");
  const date = new Date().toUTCString();
  const contentType = request.headers["content-type"] || "application/json";
  const bodyHash = request.body.length ? glint.host.call("crypto.sha256Hex", request.body) : "";
  const signature = glint.host.call("crypto.hmacSha256Hex", appKey,
    "KSO-1" + request.method + request.uri + contentType + date + bodyHash);
  return { setHeaders: {
    "Content-Type": contentType,
    "X-Kso-Date": date,
    "X-Kso-Authorization": "KSO-1 " + appId + ":" + signature
  }};
});
