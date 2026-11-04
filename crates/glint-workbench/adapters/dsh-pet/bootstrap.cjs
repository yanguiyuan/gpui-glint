// The pinned standalone app already provides the renderer and authenticated API.
// Hand its loopback endpoint back to the parent through a per-launch directory.
const { app } = require('electron');
const { mkdirSync, writeFileSync, renameSync } = require('node:fs');
const { join } = require('node:path');
const { pathToFileURL } = require('node:url');

const profile = process.env.GLINT_ADAPTER_PROFILE;
const runtime = process.env.GLINT_ADAPTER_RUNTIME;
process.env.DSH_HOME = profile;
delete process.env.DSH_PET_SITE_URL;
const userData = join(profile, 'electron');
mkdirSync(userData, { recursive: true });
app.setPath('userData', userData);

const upstream = join(process.resourcesPath, 'upstream.asar');
// The packaged executable always enters this bootstrap, including when the
// upstream helper relaunches it to probe DPI while its parent is blocked.
if (process.env.DSH_PET_DPI_PROBE === '1') {
  require(join(upstream, 'runtime', 'electron-helper', 'main.js'));
  return;
}

const ready = setInterval(() => {
  if (!process.env.DSH_PET_SITE_URL) return;
  clearInterval(ready);
  const url = new URL(process.env.DSH_PET_SITE_URL);
  const token = new URLSearchParams(url.hash.slice(1)).get('token');
  const temp = join(runtime, 'endpoint.tmp');
  writeFileSync(temp, JSON.stringify({ pid: process.pid, port: Number(url.port), token }));
  renameSync(temp, join(runtime, 'endpoint.json'));
}, 50);

import(pathToFileURL(join(upstream, 'lib', 'app.js')).href)
  .catch((error) => {
    console.error(error);
    app.exit(1);
  });
