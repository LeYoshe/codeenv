const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const http = require('node:http');
const { spawn } = require('node:child_process');
const { chromium } = require('playwright');
const binary = process.env.CODEENV_BINARY || path.resolve(__dirname, '../../target/debug/codeenv');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const children = [];
let browser, mock;
const sockets = new Set();
let root;
function start(args) {
  const child = spawn(binary, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  child.log = '';
  child.stdout.on('data', b => child.log += b);
  child.stderr.on('data', b => child.log += b);
  children.push(child);
  return child;
}
async function stop(child) {
  if (child.exitCode !== null) return;
  child.kill('SIGTERM');
  await new Promise(resolve => child.once('exit', resolve));
}
async function ready(url, child) {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(url + '/api/me')).ok) return; } catch {}
    if (child.exitCode !== null) throw new Error(child.log);
    await sleep(100);
  }
  throw new Error('Server not ready: ' + child.log);
}
function frame(kind, bytes) {
  const body = Buffer.from(bytes);
  const out = Buffer.alloc(body.length + 5);
  out.writeUInt32BE(body.length + 1);
  out[4] = kind;
  body.copy(out, 5);
  return out;
}
const json = obj => frame(1, JSON.stringify(obj));
(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'ce-review-'));
  const data = path.join(root, 'data');
  const files = path.join(root, 'files');
  await fs.mkdir(data); await fs.mkdir(files);
  await fs.writeFile(path.join(files, 'note.txt'), 'hello original\n');
  await fs.writeFile(path.join(files, 'slow.txt'), 'slow\n');
  await fs.writeFile(path.join(files, 'denied.txt'), 'denied\n');
  await fs.mkdir(path.join(files, 'folder'));
  await fs.writeFile(path.join(files, 'folder', 'child.txt'), 'child\n');
  const probe = net.createServer();
  await new Promise(r => probe.listen(0, '127.0.0.1', r));
  const port = probe.address().port;
  await new Promise(r => probe.close(r));
  const url = `http://127.0.0.1:${port}`;
  const config = path.join(root, 'config.toml');
  await fs.writeFile(config, `listen = "127.0.0.1:${port}"\nroot = ${JSON.stringify(files)}\ndata_dir = ${JSON.stringify(data)}\nshell = "/bin/bash"\n[auth]\nmode = "insecure"\nuser = "test@example.com"\n`);

  // A fake daemon deliberately splits a frame while keyboard input arrives.
  let keyboardFrames = 0;
  mock = net.createServer(socket => {
    sockets.add(socket); socket.on('close', () => sockets.delete(socket));
    let pending = Buffer.alloc(0);
    socket.on('data', chunk => {
      pending = Buffer.concat([pending, chunk]);
      while (pending.length >= 4 && pending.length >= pending.readUInt32BE() + 4) {
        const length = pending.readUInt32BE();
        const kind = pending[4];
        const payload = pending.subarray(5, length + 4);
        pending = pending.subarray(length + 4);
        if (kind === 2) { keyboardFrames++; continue; }
        const req = JSON.parse(payload);
        if (req.op === 'hello') socket.end(json({ ok: true, protocol: 1, version: 'test' }));
        if (req.op === 'list') socket.end(json({ ok: true, terminals: [{ id: 'test', owner: 'test@example.com', title: 'test', created: 0, cwd: files, command: 'bash', clients: 0, activity: 0, pid: 0 }] }));
        if (req.op === 'attach') {
          socket.write(json({ ok: true }));
          const out = frame(2, 'fragmented-output');
          socket.write(out.subarray(0, 2));
          setTimeout(() => { if (!socket.destroyed) socket.write(out.subarray(2)); }, 200);
        }
      }
    });
    socket.on('error', () => {});
  });
  await new Promise(r => mock.listen(path.join(data, 'ptyd.sock'), r));
  let server = start(['-c', config]);
  await ready(url, server);
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true });
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(url);
  const output = await page.evaluate(async () => new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://${location.host}/api/terminals/test/ws`);
    ws.binaryType = 'arraybuffer';
    const deadline = setTimeout(() => reject(new Error('fragmented frame lost')), 5000);
    let timer;
    ws.onopen = () => { timer = setInterval(() => ws.send(new Uint8Array([97])), 10); };
    ws.onmessage = e => { clearInterval(timer); clearTimeout(deadline); ws.close(); resolve(new TextDecoder().decode(e.data)); };
  }));
  assert.equal(output, 'fragmented-output'); assert(keyboardFrames > 0);
  console.log('PASS terminal output remains intact during concurrent keyboard input');
  await page.close();
  await stop(server);
  for (const socket of sockets) socket.destroy();
  await new Promise(r => mock.close(r)); mock = null;

  const daemon = start(['ptyd', '--socket', path.join(data, 'ptyd.sock'), '--scrollback', '100']);
  for (let i = 0; i < 50; i++) {
    if (daemon.log.includes('listening on')) break;
    if (daemon.exitCode !== null) throw new Error(daemon.log);
    await sleep(100);
  }
  server = start(['-c', config]); await ready(url, server);
  const ui = await browser.newPage();
  ui.on('pageerror', e => errors.push(e.message));
  await ui.goto(url);
  assert.equal(await ui.locator('html').getAttribute('lang'), 'en');
  assert.equal(await ui.locator('#buttons').getAttribute('aria-label'), 'Commands');
  assert.equal(await ui.locator('#empty-new').textContent(), 'New terminal');
  const missingTerminal = await ui.evaluate(async () => {
    const response = await fetch('/api/terminals/missing', { method: 'DELETE' });
    return { status: response.status, body: await response.json() };
  });
  assert.equal(missingTerminal.status, 404);
  assert.equal(missingTerminal.body.error, 'terminal not found');
  console.log('PASS English page language, labels, and API errors');
  await ui.locator('.node.file .name', { hasText: /^note.txt$/ }).click();
  await ui.locator('.cm-content').fill('hello edited\n');
  await ui.getByRole('button', { name: /^Save/ }).click();
  await ui.getByText('Saved', { exact: true }).waitFor();
  assert.equal(await fs.readFile(path.join(files, 'note.txt'), 'utf8'), 'hello edited\n');
  console.log('PASS browser editor saves files');

  await ui.route('**/api/fs/file?path=*slow.txt', async route => { await sleep(350); await route.continue(); });
  await ui.locator('.node.file .name', { hasText: /^slow.txt$/ }).click();
  await ui.locator('.tab.active .t-close').click();
  await sleep(600);
  assert.equal(await ui.locator('.cm-editor').count(), 1);
  console.log('PASS closing an editor during loading leaves no extra editor');

  const term = await ui.evaluate(async () => {
    const res = await fetch('/api/terminals', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ title: 'Smoke', command: "printf 'CODEENV_SMOKE\\n'" }) });
    if (!res.ok) throw new Error(await res.text());
    return res.json();
  });
  await ui.reload();
  await ui.locator('#term-list .title', { hasText: /^Smoke$/ }).click();
  await ui.locator('.tab.active:not(.connecting)').waitFor();
  await ui.evaluate(async id => new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://${location.host}/api/terminals/${id}/ws`);
    ws.binaryType = 'arraybuffer';
    let output = '';
    const timeout = setTimeout(() => { ws.close(); reject(new Error('snapshot missing expected output')); }, 5000);
    ws.onmessage = e => {
      if (typeof e.data !== 'string') output += new TextDecoder().decode(e.data);
      if (output.includes('CODEENV_SMOKE')) { clearTimeout(timeout); ws.close(); resolve(); }
    };
  }), term.id);
  await stop(server);
  server = start(['-c', config]); await ready(url, server);
  await ui.reload();
  await ui.locator('.tab.active:not(.connecting)').waitFor();
  await ui.evaluate(async id => new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://${location.host}/api/terminals/${id}/ws`);
    ws.binaryType = 'arraybuffer';
    let output = '';
    const timeout = setTimeout(() => { ws.close(); reject(new Error('snapshot missing expected output')); }, 5000);
    ws.onmessage = e => {
      if (typeof e.data !== 'string') output += new TextDecoder().decode(e.data);
      if (output.includes('CODEENV_SMOKE')) { clearTimeout(timeout); ws.close(); resolve(); }
    };
  }), term.id);
  console.log('PASS shell and screen survive a web-server restart');

  const checks = await ui.evaluate(async () => {
    const send = async (method, route, body) => {
      const r = await fetch(route, { method, headers: { 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
      return { status: r.status, text: await r.text() };
    };
    const read = await send('GET', '/api/fs/file?path=note.txt');
    const file = JSON.parse(read.text);
    const saved = await send('PUT', '/api/fs/file', { path: 'note.txt', content: 'new content', version: file.version });
    const conflict = await send('PUT', '/api/fs/file', { path: 'note.txt', content: 'stale', version: file.version });
    const search = await send('GET', '/api/search/text?dir=&q=content');
    const upload = await fetch('/api/fs/upload?dir=&path=uploaded.txt&id=smoke&done=true', { method: 'PUT', body: 'upload body' });
    const download = await send('GET', '/api/fs/download?path=uploaded.txt');
    return { saved, conflict, search, upload: upload.status, download };
  });
  assert.equal(checks.saved.status, 200); assert.equal(checks.conflict.status, 409);
  assert(checks.search.text.includes('"done":true')); assert(checks.search.text.includes('note.txt'));
  assert.equal(checks.upload, 200); assert.equal(checks.download.text, 'upload body');
  const hostStatus = await new Promise((resolve, reject) => {
    http.get(url + '/api/me', { headers: { Host: '127.example.com' } }, res => { res.resume(); resolve(res.statusCode); }).on('error', reject);
  });
  assert.equal(hostStatus, 421);
  await ui.evaluate(async id => fetch(`/api/terminals/${id}`, { method: 'DELETE' }), term.id);
  console.log('PASS conflicts, search, upload/download and Host rejection');
  await ui.route('**/api/fs/file?path=*denied.txt', route => route.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ error: 'read-only file' }) }));
  await ui.locator('.node.file .name', { hasText: /^denied.txt$/ }).click();
  await ui.getByText('read-only file', { exact: true }).waitFor();
  console.log('PASS file permission errors are not reported as expired sessions');

  await ui.locator('.node.dir .name', { hasText: /^folder$/ }).click();
  await ui.locator('.node.file .name', { hasText: /^child.txt$/ }).waitFor();
  await ui.locator('.node.dir .name', { hasText: /^folder$/ }).click({ button: 'right' });
  await ui.getByRole('menuitem', { name: /Rename/ }).click();
  await ui.locator('.rename-input').fill('renamed');
  await ui.locator('.rename-input').press('Enter');
  await ui.locator('.node.dir .name', { hasText: /^renamed$/ }).waitFor();
  await ui.locator('.node.file .name', { hasText: /^child.txt$/ }).waitFor();
  console.log('PASS renaming an expanded directory keeps its contents visible');

  await ui.locator('#panel-search .panel-toggle').click();
  await ui.locator('#search-input').fill('content');
  await ui.waitForFunction(() => document.querySelector('#search-summary').textContent.includes('1 result'));
  await ui.route('**/api/search/text?**', route => route.fulfill({ status: 200, contentType: 'application/x-ndjson', body: '' }));
  await ui.locator('#search-input').fill('interrupted');
  await ui.getByText('Search interrupted. Try again.', { exact: true }).waitFor();
  console.log('PASS content search and interrupted-stream error in the UI');
  assert.deepEqual(errors, []);
  console.log('PASS no browser JavaScript errors');
})().catch(e => { console.error(e); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close();
  for (const child of children.reverse()) await stop(child);
  for (const socket of sockets) socket.destroy();
  if (mock) await new Promise(r => mock.close(r));
  // Only the temporary directory created by this script is removed.
  if (root) await fs.rm(root, { recursive: true, force: true });
});
