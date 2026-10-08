// Record the real app with disposable sample files, never a personal workspace.
const fs = require('node:fs/promises');
const path = require('node:path');
const net = require('node:net');
const { spawn } = require('node:child_process');
const { chromium } = require('playwright');
const assert = require('node:assert/strict');

const repository = path.resolve(__dirname, '../..');
const binary = process.env.CODEENV_BINARY || path.join(repository, 'target/debug/codeenv');
const media = path.join(repository, 'site/media');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const processes = [];
let browser, temporary;

function start(args) {
  const child = spawn(binary, args, { cwd: repository, stdio: ['ignore', 'pipe', 'pipe'] });
  child.log = '';
  child.stdout.on('data', data => child.log += data);
  child.stderr.on('data', data => child.log += data);
  processes.push(child);
  return child;
}
async function waitFor(check, child) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (child.exitCode !== null) throw new Error(child.log);
    try { if (await check()) return; } catch {}
    await pause(100);
  }
  throw new Error('Demo server did not start: ' + child.log);
}

(async () => {
  temporary = await fs.mkdtemp('/tmp/codeenv-demo-');
  const workspace = path.join(temporary, 'workspace');
  const data = path.join(temporary, 'data');
  await fs.mkdir(workspace); await fs.mkdir(data); await fs.mkdir(media, { recursive: true });
  await fs.writeFile(path.join(workspace, 'README.md'), '# A small web service\n\nRun the tests, edit the greeting, and try it in your browser.\n\n```sh\npython3 -m unittest -v\npython3 server.py\n```\n');
  const source = `"""A tiny development server using the Python standard library."""\n\nfrom http.server import BaseHTTPRequestHandler, HTTPServer\n\n\ndef greeting(name="world"):\n    return f"Hello, {name}!"\n\n\nclass Handler(BaseHTTPRequestHandler):\n    def do_GET(self):\n        body = greeting().encode()\n        self.send_response(200)\n        self.send_header("Content-Type", "text/plain; charset=utf-8")\n        self.end_headers()\n        self.wfile.write(body)\n\n\nif __name__ == "__main__":\n    HTTPServer(("127.0.0.1", 8080), Handler).serve_forever()\n`;
  await fs.writeFile(path.join(workspace, 'server.py'), source);
  await fs.writeFile(path.join(workspace, 'test_server.py'), 'import unittest\nfrom server import greeting\n\nclass GreetingTests(unittest.TestCase):\n    def test_default_greeting(self):\n        self.assertEqual(greeting(), "Hello, world!")\n\n    def test_named_greeting(self):\n        self.assertEqual(greeting("codeenv"), "Hello, codeenv!")\n');
  await fs.writeFile(path.join(workspace, '.gitignore'), '__pycache__/\n');
  const shell = path.join(temporary, 'demo-shell');
  await fs.writeFile(shell, '#!/bin/sh\nexport BASH_SILENCE_DEPRECATION_WARNING=1\nexport PYTHONDONTWRITEBYTECODE=1\nexport PS1="\\[\\033[32m\\]workspace\\[\\033[0m\\] $ "\nexec /bin/bash --noprofile --norc\n', { mode: 0o755 });
  const probe = net.createServer();
  await new Promise(resolve => probe.listen(0, '127.0.0.1', resolve));
  const port = probe.address().port;
  await new Promise(resolve => probe.close(resolve));
  const url = `http://127.0.0.1:${port}`;
  const config = path.join(temporary, 'config.toml');
  await fs.writeFile(config, `listen = "127.0.0.1:${port}"\nname = "demo workspace"\nroot = ${JSON.stringify(workspace)}\ndata_dir = ${JSON.stringify(data)}\nshell = ${JSON.stringify(shell)}\n[auth]\nmode = "insecure"\nuser = "demo@example.com"\n[[buttons]]\nname = "Run tests"\ncommand = "python3 -m unittest -v"\ncolor = "#3fb950"\n`);
  const daemon = start(['ptyd', '--socket', path.join(data, 'ptyd.sock'), '--scrollback', '100']);
  await waitFor(() => daemon.log.includes('listening on'), daemon);
  const server = start(['-c', config]);
  await waitFor(async () => (await fetch(url + '/api/me')).ok, server);
  const response = await fetch(url + '/api/terminals', {
    method: 'POST', headers: { Origin: url, 'Content-Type': 'application/json' },
    body: JSON.stringify({ title: 'Project shell', command: 'python3 -m unittest -v' }),
  });
  assert(response.ok);
  const terminal = await response.json();
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true });
  // First prepare a clean initial state without recording startup screens.
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  await page.goto(url);
  await page.locator('#term-list li', { hasText: 'Project shell' }).click();
  await page.locator('.xterm-screen').waitFor();
  await pause(1400);
  await page.screenshot({ path: path.join(media, 'terminal.png') });
  await page.locator('.node.file .name', { hasText: /^server.py$/ }).click();
  await page.locator('.cm-content').waitFor();
  await pause(400);
  await page.screenshot({ path: path.join(media, 'workspace.png') });
  await context.close();

  const recording = await browser.newContext({
    viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1,
    recordVideo: { dir: path.join(temporary, 'video'), size: { width: 1440, height: 900 } },
  });
  const demo = await recording.newPage();
  const errors = [];
  demo.on('pageerror', error => errors.push(error.message));
  await demo.goto(url);
  await demo.locator('#term-list li', { hasText: 'Project shell' }).click();
  await pause(2200);
  await demo.locator('.xterm-helper-textarea').focus();
  await demo.keyboard.type('ls', { delay: 150 });
  await demo.keyboard.press('Enter');
  await pause(2200);
  await demo.locator('.node.file .name', { hasText: /^server.py$/ }).click();
  const editor = demo.locator('.cm-content');
  await editor.waitFor();
  await pause(1800);
  await editor.click();
  await demo.keyboard.press(process.platform === 'darwin' ? 'Meta+Home' : 'Control+Home');
  await demo.keyboard.type('# Edit here. Run in your terminal.\n', { delay: 65 });
  await demo.getByRole('button', { name: /^Save/ }).click();
  await demo.getByText('Saved', { exact: true }).waitFor();
  await pause(1800);
  await demo.locator('#panel-search .panel-toggle').click();
  await demo.locator('#search-input').pressSequentially('greeting', { delay: 140 });
  await demo.locator('.sr-line').first().waitFor();
  await pause(2500);
  await demo.screenshot({ path: path.join(media, 'search.png') });
  await demo.locator('#term-list li', { hasText: 'Project shell' }).click();
  await pause(2200);
  assert.deepEqual(errors, []);
  const video = demo.video();
  await recording.close();
  await video.saveAs(path.join(media, 'demo.webm'));
  await fetch(url + '/api/terminals/' + terminal.id, { method: 'DELETE', headers: { Origin: url } });
  console.log('Captured workspace.png, terminal.png, search.png, and demo.webm');
})().catch(error => { console.error(error); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close();
  for (const child of processes.reverse()) {
    if (child.exitCode !== null) continue;
    child.kill('SIGTERM');
    await Promise.race([new Promise(resolve => child.once('exit', resolve)), pause(3000)]);
    if (child.exitCode === null) child.kill('SIGKILL');
  }
  if (temporary) await fs.rm(temporary, { recursive: true, force: true });
});
