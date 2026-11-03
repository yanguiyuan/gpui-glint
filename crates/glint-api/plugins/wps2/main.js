// Matches pixeldata's WPS-2 signature action in its default (auto MD5) mode.
glint.on("request.before-send", (request) => {
  const accessKey = glint.host.call("environment.get", "wps_app_id");
  const secretKey = glint.host.call("environment.get", "wps_app_key");
  if (!accessKey || !secretKey) throw new Error("Missing WPS credentials");
  const contentType = request.headers["content-type"] || "application/json";
  const date = new Date().toUTCString();
  // Hash the original body bytes, or the encoded path and query for an empty body.
  const md5Source = request.body.trim() ? request.body : request.uri;
  const contentMd5 = glint.host.call("crypto.md5Hex", md5Source);
  const signature = glint.host.call("crypto.sha1Hex",
    secretKey + contentMd5 + contentType + date);
  return { setHeaders: {
    "Date": date,
    "Content-Type": contentType,
    "Content-MD5": contentMd5,
    "Authorization": "WPS-2:" + accessKey + ":" + signature
  }};
});
