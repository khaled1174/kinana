import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
const evidence = process.env.KINANA_UI_EVIDENCE||path.join(root, 'docs/evidence/phase-6/batch-4-5');
const binaries = Object.fromEntries(['release', 'e2e'].map(mode => [mode,
  process.env[`KINANA_${mode.toUpperCase()}_BINARY`] ?? path.join(root, `target/phase6-${mode}/release/kinana-desktop`)]));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', maxBuffer: 128 * 1024 * 1024 });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}
function bundle(dir) { return readdirSync(dir, { recursive: true }).filter(p => /\.js$/.test(p)).map(p => readFileSync(path.join(dir, p), 'utf8')).join('\n'); }
const signature = /wdioTauri|__wdio_mocks__|\[WDIO Plugin\]/;
assert.ok(!signature.test(bundle(path.join(root, 'app/dist'))), 'default frontend contains WDIO');
assert.ok(signature.test(bundle(path.join(root, 'app/dist-e2e'))), 'positive-control frontend lacks WDIO');
for (const marker of ['__kinanaE2EUnsafe', '__kinanaTrustedSelfLoaded']) {
  assert.equal(bundle(path.join(root, 'app/dist')).includes(marker), false, `release contains ${marker}`);
  assert.equal(bundle(path.join(root, 'app/dist-e2e')).includes(marker), true, `e2e lacks ${marker}`);
}
assert.equal(readdirSync(path.join(root, 'app/dist')).includes('e2e-csp-self-probe.js'), false);
assert.equal(readdirSync(path.join(root, 'app/dist-e2e')).includes('e2e-csp-self-probe.js'), true);
const report = { csp_test_hooks: {release_present:false,e2e_present:true}, frontend: { release_has_wdio: false, e2e_has_wdio: true }, binaries: {}, runtime: {} };
for (const [mode, binary] of Object.entries(binaries)) {
  const symbols = run('/usr/bin/nm', ['-a', binary]);
  const strings = run('/usr/bin/strings', ['-a', binary]);
  const checks = Object.fromEntries(['tauri_plugin_wdio_webdriver', 'tauri_plugin_wdio'].map(name => {
    const pattern = name === 'tauri_plugin_wdio' ? /tauri_plugin_wdio(?!_webdriver)/ : /tauri_plugin_wdio_webdriver/;
    const symbolPresent = pattern.test(symbols);
    const stringPresent = pattern.test(strings);
    assert.equal(symbolPresent, mode === 'e2e', `${mode}: unexpected symbols for ${name}`);
    assert.equal(stringPresent, mode === 'e2e', `${mode}: unexpected strings for ${name}`);
    return [name, { symbol_present: symbolPresent, string_present: stringPresent }];
  }));
  for (const marker of ['KINANA_E2E_HOME','SKILLSBOX_E2E_HOME']) {
    assert.equal(strings.includes(marker), mode === 'e2e', `${mode}: unexpected test home override ${marker}`);
  }
  report.binaries[mode] = { binary, sha256: createHash('sha256').update(readFileSync(binary)).digest('hex'), plugins: checks };
  console.log(`PASS ${mode}: native plugin symbols/strings ${mode === 'e2e' ? 'present (positive control)' : 'absent'}`);
}
const graph = run('cargo', ['tree', '--locked', '-p', 'kinana-desktop', '--edges', 'normal,build']);
assert.ok(!/tauri-plugin-wdio/.test(graph), 'default Cargo graph includes WDIO');
writeFileSync(path.join(evidence, 'release-dependency-tree.txt'), graph);
async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
async function isOpen(port) {
  return new Promise(resolve => {
    const socket = net.connect({ host: '127.0.0.1', port });
    socket.setTimeout(200);
    socket.on('connect', () => { socket.destroy(); resolve(true); });
    socket.on('error', () => resolve(false));
    socket.on('timeout', () => { socket.destroy(); resolve(false); });
  });
}
async function checkRuntime(mode) {
  const home = mkdtempSync(path.join(tmpdir(), 'kinana-isolation-'));
  const port = await freePort();
  const child = spawn(binaries[mode], [], { env: { ...process.env, HOME: home, TAURI_WEBDRIVER_PORT: String(port) }, stdio: ['ignore', 'pipe', 'pipe'] });
  const output = [];
  child.stdout.on('data', data => output.push(data)); child.stderr.on('data', data => output.push(data));
  child.on('error', error => { output.push(String(error)); });
  let samples = 0;
  try {
    let opened = false;
    for (let i = 0; i < 40; i++) {
      assert.equal(child.exitCode, null, `${mode} exited: ${output.join('')}`);
      assert.equal(child.signalCode, null, `${mode} killed during startup`);
      const listening = await isOpen(port);
      samples++;
      if (mode === 'release') assert.equal(listening, false, 'default release opened the requested WebDriver port');
      else if (listening) { opened = true; break; }
      await delay(200);
    }
    // Observe an actual WindowServer window owned by this process, not merely a living process.
    const windowJSON = run('/usr/bin/swift', ['-e', `import CoreGraphics; import Foundation; let windows = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String:Any]] ?? []; let owned = windows.filter { ($0[kCGWindowOwnerPID as String] as? Int) == ${child.pid} }; let data = try! JSONSerialization.data(withJSONObject: owned); print(String(data: data, encoding: .utf8)!)`]);
    const windows = JSON.parse(windowJSON);
    const mainWindow = windows.find(w => w.kCGWindowLayer === 0 && w.kCGWindowBounds.Width >= 800 && w.kCGWindowBounds.Height >= 600);
    assert.ok(mainWindow, `${mode}: no native application window`);
    const sockets = spawnSync('/usr/sbin/lsof', ['-Pan', '-p', String(child.pid), '-iTCP', '-sTCP:LISTEN'], { encoding: 'utf8' });
    if (mode === 'release') {
      assert.equal(sockets.status, 1, `default release owns a TCP listener: ${sockets.stdout}`);
      assert.equal(sockets.stdout.trim(), '');
      assert.equal(await isOpen(4445), false, 'default WebDriver port 4445 must also be closed');
    } else {
      assert.ok(opened, 'positive-control e2e server never opened');
      assert.match(sockets.stdout, new RegExp(`127.0.0.1:${port}.*LISTEN`));
      const status = await (await fetch(`http://127.0.0.1:${port}/status`)).json();
      assert.equal(status.value.ready, true);
    }
    report.runtime[mode] = { pid: child.pid, requested_port: port, observation_samples: samples, window_count: windows.length, native_window_bounds: windows.map(w => w.kCGWindowBounds), tcp_listeners: sockets.stdout, webdriver_open: mode === 'e2e', process_alive: child.exitCode === null };
    console.log(`PASS ${mode}: native window present, process alive, ${mode === 'e2e' ? 'loopback WebDriver opens (positive control)' : 'no WebDriver port and no TCP listeners'}`);
  } finally {
    child.kill('SIGTERM');
    await Promise.race([new Promise(resolve => child.once('exit', resolve)), delay(3000)]);
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    writeFileSync(path.join(evidence, `isolation-${mode}-app.txt`), output.join(''));
    rmSync(home, { recursive: true, force: true });
  }
}
await checkRuntime('release');
await checkRuntime('e2e');
writeFileSync(path.join(evidence, 'release-isolation.json'), JSON.stringify(report, null, 2)+'\n');
console.log('PASS default release isolation; rerun this acceptance check for the final phase-7 artifact');
