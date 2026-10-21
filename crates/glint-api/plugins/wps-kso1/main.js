// glint-api owns the request contract; the shared plugin runtime knows no HTTP algorithms.
glint.on("request.before-send", (request) => {
  const appId = glint.host.call("environment.get", "wps_app_id");
  const appKey = glint.host.call("environment.get", "wps_app_key");
  if (!appId || !appKey) throw new Error("Missing WPS credentials");
  const date = new Date().toUTCString();
  const contentType = request.headers["content-type"] || "application/json";
  const bodyHash = request.body.length ? glint.host.call("crypto.sha256Hex", request.body) : "";
  // Event WebSocket authentication uses a fixed path, excluding gateway prefixes.
  const eventPath = "/v7/event/ws";
  const isWebSocket = request.scheme === "ws" || request.scheme === "wss";
  const isEventWebSocket = isWebSocket
    && request.uri.split("?")[0].endsWith(eventPath);
  let signingUri = isEventWebSocket ? eventPath : request.uri;
  if (!isWebSocket) {
    // Private OpenAPI gateways remove the deployment prefix and /openapi.
    // Match the path only; preserve query order and percent encoding verbatim.
    const queryStart = request.uri.indexOf("?");
    const path = queryStart < 0 ? request.uri : request.uri.slice(0, queryStart);
    const apiPath = path.match(/^.*?\/openapi(\/v\d+(?:\/.*)?)$/);
    if (apiPath) {
      signingUri = apiPath[1] + (queryStart < 0 ? "" : request.uri.slice(queryStart));
    }
  }
  const signature = glint.host.call("crypto.hmacSha256Hex", appKey,
    "KSO-1" + request.method + signingUri + contentType + date + bodyHash);
  return { setHeaders: {
    "Content-Type": contentType,
    "X-Kso-Date": date,
    "X-Kso-Authorization": "KSO-1 " + appId + ":" + signature
  }};
});
