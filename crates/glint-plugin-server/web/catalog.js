'use strict';
const $ = (id) => document.getElementById(id);
const base = new URL('./', window.location.href);
const sourceUrl = base.href.replace(/\/$/, '');
const skillUrl = new URL('skills/glint-plugin-dev/SKILL.md', base).href;
let theme = 'light';
try { theme = localStorage.getItem('glint-catalog-theme') || 'light'; } catch {}
document.documentElement.dataset.theme = theme === 'dark' ? 'dark' : 'light';
$('theme-toggle').addEventListener('click', () => {
  const next = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
  document.documentElement.dataset.theme = next;
  $('theme-toggle').setAttribute('aria-label', next === 'dark' ? '切换浅色外观' : '切换深色外观');
  try { localStorage.setItem('glint-catalog-theme', next); } catch {}
});
for (const id of ['open-source','connect-source']) $(id).addEventListener('click', () => $('source-dialog').showModal());
$('close-source').addEventListener('click', () => $('source-dialog').close());
$('source-dialog').addEventListener('click', e => { if (e.target === $('source-dialog')) { const r = $('source-dialog').getBoundingClientRect(); if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) $('source-dialog').close(); } });
$('source-host').textContent = new URL(base).host;
const state = { info: null, releases: [], busy: false, selected: [], softwareBusy:false, toastTimer: null };
const node = (tag, text, cls) => { const el = document.createElement(tag); if (text !== undefined) el.textContent = text; if (cls) el.className = cls; return el; };
const alias = base.hostname.split('.')[0].replace(/[^a-z0-9_-]/g, '') || 'glint';
$('source-url').value = sourceUrl;
function toast(message) { $('toast').textContent = message; $('toast').hidden = false; clearTimeout(state.toastTimer); state.toastTimer = setTimeout(() => $('toast').hidden = true, 3500); }
async function copy(text) { try { await navigator.clipboard.writeText(text); toast('已复制'); } catch { toast('无法访问剪贴板，请选中文字手动复制。'); } }
function quote(text) { return "'" + text.replace(/'/g, "'\\''") + "'"; }
function powershellQuote(text) { return "'" + text.replace(/'/g, "''") + "'"; }
function installCommand() {
  if ($('platform').value === 'windows') return '$skillsHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $HOME ".codex" }\n$skillDir = Join-Path $skillsHome "skills/glint-plugin-dev"\nNew-Item -ItemType Directory -Force $skillDir | Out-Null\nInvoke-WebRequest -Uri ' + powershellQuote(skillUrl) + ' -OutFile (Join-Path $skillDir "SKILL.md")';
  return 'skill_dir="${CODEX_HOME:-$HOME/.codex}/skills/glint-plugin-dev"\nmkdir -p "$skill_dir"\ncurl --fail --show-error --location --proto "=https" --tlsv1.2 ' + quote(skillUrl) + ' -o "$skill_dir/SKILL.md"';
}
function renderInstall() { $('install-command').textContent = installCommand(); }
$('platform').addEventListener('change', renderInstall);
$('copy-install').addEventListener('click', () => copy(installCommand()));
$('copy-source').addEventListener('click', () => copy(JSON.stringify({ name: alias, url: sourceUrl, public_key: state.info.public_key }, null, 2)));
$('copy-cli').addEventListener('click', () => copy('glint-pluginctl source add ' + alias + ' ' + quote(sourceUrl) + ' --public-key ' + quote(state.info.public_key)));
renderInstall();
// Compare numeric identifiers without converting possibly large SemVer components to floats.
function numberCompare(a, b) { return a.length - b.length || (a < b ? -1 : a > b ? 1 : 0); }
function versionCompare(a, b) {
  const parts = v => { const text = v.split('+')[0], i = text.indexOf('-'); return i < 0 ? [text, undefined] : [text.slice(0, i), text.slice(i + 1)]; };
  const [ac, ap] = parts(a); const [bc, bp] = parts(b);
  const aa = ac.split('.'), bb = bc.split('.');
  for (let i = 0; i < 3; i++) { const diff = numberCompare(aa[i], bb[i]); if (diff) return diff; }
  if (ap === undefined || bp === undefined) return ap === bp ? 0 : ap === undefined ? 1 : -1;
  const ai = ap.split('.'), bi = bp.split('.');
  for (let i = 0; i < Math.max(ai.length, bi.length); i++) {
    if (ai[i] === undefined || bi[i] === undefined) return ai[i] === undefined ? -1 : 1;
    const an = /^\d+$/.test(ai[i]), bn = /^\d+$/.test(bi[i]);
    const diff = an && bn ? numberCompare(ai[i], bi[i]) : an !== bn ? (an ? -1 : 1) : ai[i] < bi[i] ? -1 : ai[i] > bi[i] ? 1 : 0;
    if (diff) return diff;
  }
  return 0;
}
function packageLink(release, cls) {
  if (!/^[a-f0-9]{64}$/.test(release.sha256)) throw new Error('插件包校验信息无效。');
  const link = node('a', '下载插件包 ↓', cls);
  link.href = new URL('v1/packages/' + release.sha256, base).href;
  link.download = release.manifest.id + '-' + release.manifest.version + '.glintpkg';
  return link;
}
function grouped() {
  const groups = new Map();
  for (const r of state.releases) {
    if (!groups.has(r.manifest.id)) groups.set(r.manifest.id, []);
    groups.get(r.manifest.id).push(r);
  }
  return [...groups.values()].map(group => group.sort((a, b) => versionCompare(b.manifest.version, a.manifest.version))).filter(group => group.some(r => !r.yanked));
}
function render() {
  const query = $('search').value.trim().toLocaleLowerCase(); const host = $('host-filter').value;
  const groups = grouped(); const matching = groups.filter(group => group.some(r => !r.yanked && (!host || r.hosts.some(h => h.id === host)) &&
    [r.manifest.name, r.manifest.id, r.manifest.version, r.changelog, ...r.manifest.capabilities, ...r.hosts.map(h => h.id)].join(' ').toLocaleLowerCase().includes(query)));
  $('plugin-count').textContent = String(groups.length);
  $('results-summary').textContent = `共 ${groups.length} 个可用插件${query || host ? `，找到 ${matching.length} 个匹配` : ''} · 安装后在客户端启用`;
  $('plugins').replaceChildren(); $('empty').hidden = matching.length !== 0; $('error').hidden = true;
  const fragment = document.createDocumentFragment();
  for (const group of matching) {
    const release = group.find(r => !r.yanked && (!host || r.hosts.some(h => h.id === host))); const manifest = release.manifest;
    const card = node('article', undefined, 'plugin-card'); card.append(node('div', '{ }', 'plugin-symbol'));
    const content = node('div'); const title = node('div', undefined, 'card-title'); title.append(node('h3', manifest.name), node('span', 'v' + manifest.version, 'pill'));
    content.append(title, node('p', manifest.id, 'mono muted'), node('p', release.changelog || '发布者未提供版本说明。', 'card-description'));
    const bottom = node('div', undefined, 'card-bottom'); const hosts = node('div', undefined, 'host-tags');
    for (const host of release.hosts) hosts.append(node('span', host.id, 'host-tag'));
    const actions = node('div', undefined, 'card-actions'); const details = node('button', '查看详情 →', 'details-button'); details.addEventListener('click', () => openDetails(group, release.manifest.version));
    actions.append(details, packageLink(release)); bottom.append(hosts, actions); content.append(bottom); card.append(content); fragment.append(card);
  }
  $('plugins').append(fragment);
}
function openDetails(group, version) {
  state.selected = group; $('detail-title').textContent = group[0].manifest.name; $('detail-id').textContent = group[0].manifest.id;
  $('version').replaceChildren(...group.map(r => { const option = node('option', r.manifest.version + (r.yanked ? ' · 已撤回' : '')); option.value = r.manifest.version; return option; }));
  $('version').value = version || group.find(r => !r.yanked).manifest.version; renderDetail(); $('details').showModal();
}
function detailSection(title, children) { const section = node('section'); section.append(node('h3', title), ...children); return section; }
function renderDetail() {
  const release = state.selected.find(r => r.manifest.version === $('version').value); const manifest = release.manifest; const body = $('detail-body'); body.replaceChildren();
  body.append(detailSection('版本说明', [node('p', release.changelog || '暂无版本说明。', 'changelog')]));
  const hosts = node('ul'); for (const h of release.hosts) hosts.append(node('li', h.id + ' ≥ ' + h.min_version)); body.append(detailSection('适用应用', [hosts]));
  const capabilities = node('ul'); for (const permission of manifest.capabilities) capabilities.append(node('li', permission));
  body.append(detailSection('所需权限', manifest.capabilities.length ? [capabilities] : [node('p', '未申请宿主权限。', 'muted')]));
  const contributions = node('ul'); for (const c of manifest.contributions || []) contributions.append(node('li', c.title + ' · ' + c.kind));
  if ((manifest.contributions || []).length) body.append(detailSection('提供的能力', [contributions]));
  for (const contribution of manifest.contributions || []) {
    if (contribution.data !== undefined) {
      const description = contribution.kind === 'oauth.preset' ? '授权预设 · ' + contribution.title : '声明式配置 · ' + contribution.title;
      body.append(detailSection(description, [node('pre', JSON.stringify(contribution.data, null, 2), 'hash mono')]));
    }
  }
  body.append(detailSection('SHA-256', [node('p', release.sha256, 'hash mono')]));
  if (release.yanked) body.append(node('p', '此版本已撤回，请选择其他版本。', 'muted')); else body.append(packageLink(release, 'button primary'));
}
$('version').addEventListener('change', renderDetail); $('close-details').addEventListener('click', () => $('details').close());
$('details').addEventListener('click', e => { if (e.target === $('details')) { const rect = $('details').getBoundingClientRect(); if (e.clientX < rect.left || e.clientX > rect.right || e.clientY < rect.top || e.clientY > rect.bottom) $('details').close(); } });
$('search').addEventListener('input', render); $('host-filter').addEventListener('change', render); $('refresh').addEventListener('click', load);
async function getJson(path, limit) {
  const response = await fetch(new URL(path, base), { cache: 'no-store', signal: AbortSignal.timeout(15000) });
  if (!response.ok) throw new Error('插件源暂时不可用，请稍后刷新。');
  const bytes = await response.arrayBuffer(); if (bytes.byteLength > limit) throw new Error('插件目录超出大小限制。');
  return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
}
function unbase64(text) { return Uint8Array.from(atob(text), c => c.charCodeAt(0)); }
async function load() {
  if (state.busy) return; state.busy = true; $('refresh').disabled = true;
  try {
    const [info, signed] = await Promise.all([getJson('v1/info', 8192), getJson('v1/index', 6 * 1024 * 1024)]);
    const payload = unbase64(signed.payload); const index = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(payload));
    let verified = false;
    try {
      const key = await crypto.subtle.importKey('raw', unbase64(info.public_key), 'Ed25519', false, ['verify']);
      verified = await crypto.subtle.verify('Ed25519', key, unbase64(signed.signature), payload);
      if (!verified) throw new Error('目录签名校验失败。');
    } catch (error) { if (error.name !== 'NotSupportedError') throw error; }
    if (index.source_id !== info.source_id || index.format !== 1 || index.expires_at * 1000 < Date.now() || !Array.isArray(index.releases)) throw new Error('插件目录已过期或源信息不匹配，请刷新重试。');
    state.info = info; state.releases = index.releases; $('source-key').value = info.public_key;
    $('copy-source').disabled = false; $('copy-cli').disabled = false;
    $('signature').textContent = verified ? '● 已使用此源公钥校验目录签名' : '此浏览器未提供 Ed25519 校验，请使用客户端验证。';
    const filter = $('host-filter').value; $('host-filter').replaceChildren(node('option', '所有应用')); $('host-filter').firstChild.value = '';
    for (const host of [...new Set(index.releases.flatMap(r => r.hosts.map(h => h.id)))].sort()) { const option = node('option', host); option.value = host; $('host-filter').append(option); }
    if ([...$('host-filter').options].some(o => o.value === filter)) $('host-filter').value = filter;
    render();
  } catch (error) {
    state.releases = []; state.info = null; $('plugins').replaceChildren(); $('empty').hidden = true; $('error').hidden = false;
    $('error').textContent = '无法加载插件目录。' + (error instanceof Error && error.message.length < 120 ? error.message : '请刷新重试。');
    $('results-summary').textContent = '目录加载失败'; $('signature').textContent = '暂未验证源信息'; $('plugin-count').textContent = '—'; $('copy-source').disabled = true; $('copy-cli').disabled = true;
  } finally { state.busy = false; $('refresh').disabled = false; }
}

function updateNavigation() {
  const development = location.hash === '#develop';
  const software = location.hash === '#software';
  $('catalog').hidden = development || software;
  $('develop').hidden = !development;
  $('software').hidden = !software;
  if (software) loadSoftware();
  document.title = development ? '开发插件 · Glint' : software ? '软件下载 · Glint' : '插件目录 · Glint';
  window.scrollTo(0, 0);
  for (const link of document.querySelectorAll('nav a')) {
    const active = link.getAttribute('href') === (['#develop','#software'].includes(location.hash) ? location.hash : '#catalog');
    link.classList.toggle('active', active);
    if (active) link.setAttribute('aria-current', 'page'); else link.removeAttribute('aria-current');
  }
}
addEventListener('hashchange', updateNavigation); updateNavigation();

load();

$('refresh-software').addEventListener('click',loadSoftware);
async function loadSoftware() {
  if(state.softwareBusy) return;state.softwareBusy=true;$('refresh-software').disabled=true;
  try {
    const info=await getJson('v1/info',8192),apps=await getJson('v1/apps',8192);
    if(!Array.isArray(apps) || apps.length>32) throw new Error('软件目录格式无效');
    const key=await crypto.subtle.importKey('raw',unbase64(info.public_key),'Ed25519',false,['verify']);
    const feeds=await Promise.all(apps.map(async app=>{
      if(!/^[a-z0-9_-]{1,64}$/.test(app)) throw new Error('软件 ID 无效');
      const signed=await getJson('v1/apps/'+app+'/releases',6*1024*1024),bytes=unbase64(signed.payload);
      if(!await crypto.subtle.verify('Ed25519',key,unbase64(signed.signature),bytes)) throw new Error('版本签名校验失败');
      const feed=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));
      if(feed.app_id!==app || feed.source_id!==info.source_id || feed.format!==1 || feed.expires_at*1000<Date.now() || !Array.isArray(feed.releases)) throw new Error('版本目录过期或无效');
      return feed;
    }));
    $('software-list').replaceChildren();let total=0;
    for(const feed of feeds){
      const releases=feed.releases.filter(r=>!r.yanked).sort((a,b)=>versionCompare(b.version,a.version));if(!releases.length)continue;total++;
      const card=node('article',undefined,'software-card'),head=node('div',undefined,'software-card-head');
      head.append(node('h2',feed.app_id==='glint-api'?'Glint API':feed.app_id));
      const select=node('select');select.setAttribute('aria-label',feed.app_id+' 版本');
      for(const release of releases){const option=node('option',release.version);option.value=release.version;select.append(option);}head.append(select);card.append(head);
      const body=node('div');card.append(body);
      function renderVersion(){
        const release=releases.find(r=>r.version===select.value);body.replaceChildren();
        body.append(node('p',release.notes || '暂无版本说明。','changelog'));
        for(const artifact of release.artifacts){
          if(!/^[a-f0-9]{64}$/.test(artifact.sha256) || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,159}$/.test(artifact.name)) throw new Error('安装包信息无效');
          const row=node('div',undefined,'software-artifact'),detail=node('div');
          detail.append(node('p',({'x86_64-pc-windows-msvc':'Windows · x64','aarch64-apple-darwin':'macOS · Apple Silicon','x86_64-apple-darwin':'macOS · Intel'}[artifact.target] || artifact.target)+' · '+(artifact.size/1048576).toFixed(1)+' MB','artifact-label'),node('p',artifact.sha256,'hash mono'));
          const link=node('a','下载安装包 ↓','button primary');link.href=new URL('v1/apps/'+feed.app_id+'/assets/'+artifact.sha256+'/'+artifact.name,base).href;link.download=artifact.name;row.append(detail,link);body.append(row);
        }
      }
      select.addEventListener('change',renderVersion);renderVersion();$('software-list').append(card);
    }
    $('software-status').textContent=total ? '共 '+total+' 个软件 · 已使用此源公钥验证版本目录' : '暂时没有发布的软件版本。';
  } catch(error){$('software-status').textContent='无法获取软件版本，请刷新重试。';$('software-list').replaceChildren();}
  finally{state.softwareBusy=false;$('refresh-software').disabled=false;}
}
