#!/usr/bin/env node
/** Linux packaged Electron process ownership, not a visual or model smoke. */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import net from 'node:net';
import { randomInt, randomUUID } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export function requireCleanExit(result, label) {
  assert.equal(result.code, 0, `${label} exit code: ${result.code}`);
  assert.equal(result.signal, null, `${label} was signalled: ${result.signal}`);
}
export function exitReceipt(child) {
  return new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => resolve({ code, signal }));
    if (child.exitCode !== null || child.signalCode !== null)
      resolve({ code: child.exitCode, signal: child.signalCode });
  });
}
export async function deadline(promise, ms, label) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out`)), ms);
  })]); } finally { clearTimeout(timer); }
}
async function listening(port) {
  return new Promise((resolve) => {
    const socket = net.connect({ host: '127.0.0.1', port });
    const finish = (yes) => { socket.destroy(); resolve(yes); };
    socket.once('connect', () => finish(true));
    socket.once('error', () => finish(false));
    socket.setTimeout(500, () => finish(false));
  });
}
async function portsForRun() {
  for (let attempt = 0; attempt < 30; attempt++) {
    const offset = randomInt(10000, 45000);
    const ports = [7890, 7891, 7895, 7897].map((p) => p + offset);
    const held = [];
    try {
      for (const port of ports) {
        const server = net.createServer();
        await new Promise((resolve, reject) => {
          server.once('error', reject);
          server.listen(port, '127.0.0.1', resolve);
        });
        held.push(server);
      }
      return { offset, ports };
    } catch { /* Try another complete private port set. */ }
    finally { await Promise.all(held.map((s) => new Promise((resolve) => s.close(resolve)))); }
  }
  throw new Error('Cannot reserve four isolated Electron ports');
}
export function isolatedEnvironment(root, offset, inherited = process.env) {
  const env = {};
  for (const key of ['PATH', 'DISPLAY', 'XAUTHORITY', 'LANG', 'LC_ALL'])
    if (inherited[key]) env[key] = inherited[key];
  for (const [key, leaf] of Object.entries({ HOME: 'home', USERPROFILE: 'home',
    XDG_CONFIG_HOME: 'config', APPDATA: 'config', XDG_DATA_HOME: 'data',
    LOCALAPPDATA: 'data', XDG_CACHE_HOME: 'cache', XDG_STATE_HOME: 'state', TMPDIR: 'tmp' })) {
    env[key] = path.join(root, leaf); fs.mkdirSync(env[key], { recursive: true });
  }
  return { ...env, WORKSPACER_PORT_OFFSET: String(offset), WORKSPACER_USAGE_POLL_ON_BOOT: '0' };
}
async function jsonGet(port, token, suffix = '/health') {
  const response = await fetch(`http://127.0.0.1:${port}${suffix}`, {
    headers: { Authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(5000) });
  assert.equal(response.status, 200);
  return response.json();
}
async function eventually(check, label) {
  const end = Date.now() + 45000;
  let last;
  while (Date.now() < end) {
    try { return await check(); } catch (error) { last = error; }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`${label}: ${last?.message}`);
}
async function busCall(port, token, method) {
  const { default: WebSocket } = await import('ws');
  const socket = new WebSocket(`ws://127.0.0.1:${port}/bus?token=${encodeURIComponent(token)}`);
  try {
    return await deadline(new Promise((resolve, reject) => {
      socket.once('error', reject);
      socket.on('message', (bytes) => {
        const frame = JSON.parse(bytes.toString());
        if (frame.op === 'hello') socket.send(JSON.stringify({ op: 'call', id: 'probe', method, params: {} }));
        if (frame.id === 'probe' && frame.op === 'result') resolve(frame.result);
        if (frame.id === 'probe' && frame.op === 'error') reject(new Error(frame.error));
      });
    }), 5000, method);
  } finally { socket.terminate(); }
}
export function checkFacadeHealth(value, hub, mcp) {
  const expected = { status: 'ok', service: 'workspacer-mcp-facade', implementation: 'rust',
    hubConnected: true, pluginCatalogReady: true, listenAddr: `127.0.0.1:${mcp}`,
    hubUrl: `ws://127.0.0.1:${hub}/bus` };
  for (const [key, expectedValue] of Object.entries(expected))
    assert.equal(value[key], expectedValue, `MCP health ${key}`);
}
function catalogNames(body) {
  assert.equal(body.jsonrpc, '2.0'); assert.equal(body.id, 1); assert.equal(body.error, undefined);
  const result = body.result;
  assert.equal(result.resultType, 'complete'); assert.equal(result.ttlMs, 0);
  assert.equal(result.cacheScope, 'private');
  assert(Array.isArray(result.tools), 'missing tools array');
  const names = result.tools.map((tool) => tool.name);
  assert(names.every((name) => typeof name === 'string' && name.length > 0), 'invalid tool name');
  assert.equal(new Set(names).size, names.length, 'duplicate tool names');
  return names;
}
export function checkModernCatalog(body) {
  const names = catalogNames(body);
  // Captured operator fixture currently contains100 unique built-in tools.
  assert(names.length >= 100, 'operator catalog below retained100-tool floor');
  assert(names.includes('get_host_cwd'), 'missing real catalog floor');
  return names.length;
}
export function checkDetachedCatalog(body) {
  const names = catalogNames(body);
  assert(names.includes('help'), 'external facade lost its local help tool');
  assert(!names.includes('get_host_cwd'), 'desktop capability remains after provider quit');
  return names.length;
}
export function checkHelpResult(body) {
  assert.equal(body.jsonrpc, '2.0'); assert.equal(body.id, 1); assert.equal(body.error, undefined);
  assert(body.result && body.result.isError !== true, 'external help call failed');
  assert(Array.isArray(body.result.content) && body.result.content.some((item) =>
    item.type === 'text' && typeof item.text === 'string' && item.text.trim().length > 0),
  'external help returned no text');
}
async function modernRequest(port, token, method, params = {}) {
  const version = '2026-07-28';
  const response = await fetch(`http://127.0.0.1:${port}/mcp`, { method: 'POST',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json',
      Accept: 'application/json, text/event-stream', 'MCP-Protocol-Version': version, 'Mcp-Method': method,
      ...(method === 'tools/call' ? { 'Mcp-Name': params.name } : {}) },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: { ...params, _meta: {
      'io.modelcontextprotocol/protocolVersion': version,
      'io.modelcontextprotocol/clientInfo': { name: 'electron-ownership-smoke', version: '1' },
      'io.modelcontextprotocol/clientCapabilities': {} } } }), signal: AbortSignal.timeout(5000) });
  assert.equal(response.status, 200); return response.json();
}
async function modernCatalog(port, token) {
  return checkModernCatalog(await modernRequest(port, token, 'tools/list'));
}
export async function detachedFacade(port, token) {
  // The desktop provider is gone, but the external hub and its local facade
  // remain owned elsewhere. Observe unregistration before testing that facade.
  const catalogSize = checkDetachedCatalog(await modernRequest(port, token, 'tools/list'));
  checkHelpResult(await modernRequest(port, token, 'tools/call', { name: 'help', arguments: {} }));
  return catalogSize;
}
async function normalQuit(app, exited) {
  // Do not use Playwright close(): its outer cleanup may force-kill on failure.
  // Scheduling leaves time for the evaluate reply; normal Electron before-quit
  // handlers still own all asynchronous shutdown work.
  await app.evaluate(({ app }) => { setTimeout(() => app.quit(), 25); });
  const status = await deadline(exited, 20000, 'normal Electron app.quit');
  requireCleanExit(status, 'Electron'); return status;
}
export async function smoke(executable, output) {
  assert.equal(process.platform, 'linux', 'this probe owns Linux/Xvfb only');
  assert(path.isAbsolute(executable) && fs.statSync(executable).isFile(), 'required unpacked executable missing');
  const resources = path.join(path.dirname(executable), 'resources');
  const backend = path.join(resources, 'hub/workspacer-rust');
  for (const item of [backend, path.join(resources, 'claudemon/claudemon'), path.join(resources, 'app.asar')])
    assert(fs.statSync(item).isFile(), `packaged resource missing: ${item}`);
  const { _electron } = await import('playwright');
  const results = [];
  for (const adopted of [false, true]) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-electron-owned-'));
    const { offset, ports } = await portsForRun();
    const [hook, api, hub, mcp] = ports;
    const env = isolatedEnvironment(root, offset);
    const token = randomUUID(); const marker = randomUUID();
    const config = path.join(env.XDG_CONFIG_HOME, 'workspacer');
    fs.mkdirSync(config, { recursive: true });
    fs.writeFileSync(path.join(config, 'remote-token'), token, { mode: 0o600 });
    fs.writeFileSync(path.join(config, 'config.yaml'), `agents:\n  checkProviderOnStartup: false\nusage:\n  pollOnBoot: false\nupdates:\n  enabled: false\npluginSettings:\n  ownershipMarker: ${marker}\n`);
    let app, child, exited, external, externalExit;
    let diagnostics = '';
    let detachedCatalogSize;
    const capture = (data) => { diagnostics = (diagnostics + String(data).replaceAll(token, '[fixture-token]')).slice(-32000); };
    try {
      if (adopted) {
        // This external process owns only the control plane. Electron starts
        // its own daemon later; pre-borrowing that daemon would require it to
        // be healthy before Electron has even launched.
        const args = ['serve', '--config-dir', config, '--data-dir', path.join(root, 'external'),
          '--host', '127.0.0.1', '--hub-port', String(hub), '--hub-only', '--quiet',
          '--mcp-port', String(mcp), '--no-claudemon-init'];
        // Exercise the packaged parser before waiting for a service that a bad
        // argument vector could never start. --help has no startup effects.
        const parsed = spawnSync(backend, [...args, '--help'], { env, encoding: 'utf8', timeout: 5000 });
        capture(parsed.stderr ?? '');
        if (parsed.error) throw parsed.error;
        requireCleanExit({ code: parsed.status, signal: parsed.signal }, 'external Rust argument preflight');
        external = spawn(backend, args,
        { env: { ...env, HUB_TOKEN: token, WORKSPACER_PARENT_PID: String(process.pid) }, stdio: ['pipe', 'pipe', 'pipe'] });
        externalExit = exitReceipt(external);
        external.stdout.on('data', capture); external.stderr.on('data', capture);
        await eventually(() => jsonGet(hub, token), 'external Rust ready');
      }
      app = await _electron.launch({ executablePath: executable,
        args: ['--no-sandbox', '--disable-gpu'], env, timeout: 45000 });
      child = app.process(); exited = exitReceipt(child);
      child.stdout?.on('data', capture); child.stderr?.on('data', capture);
      const identity = await app.evaluate(({ app }) => ({ packaged: app.isPackaged, userData: app.getPath('userData') }));
      assert.equal(identity.packaged, true); assert(identity.userData.startsWith(root + path.sep), 'Electron escaped isolated profile');
      await eventually(async () => {
        const health = await jsonGet(hub, token); assert.equal(health.status, 'ok');
        const value = await busCall(hub, token, 'config.get');
        assert.equal(value.pluginSettings?.ownershipMarker, marker);
      }, 'desktop-provided config.get');
      await eventually(async () => checkFacadeHealth(await jsonGet(mcp, token), hub, mcp), 'Rust MCP identity');
      const catalogSize = await eventually(() => modernCatalog(mcp, token), 'modern MCP catalog');
      assert(await listening(hook) && await listening(api), 'Electron claudemon listeners missing');
      const status = await normalQuit(app, exited);
      assert(!(await listening(hook)) && !(await listening(api)), 'Electron-owned daemon survived normal quit');
      if (adopted) {
        assert.equal(external.exitCode, null); assert.equal(external.signalCode, null);
        assert.equal((await jsonGet(hub, token)).status, 'ok');
        checkFacadeHealth(await jsonGet(mcp, token), hub, mcp);
        detachedCatalogSize = await eventually(() => detachedFacade(mcp, token), 'external facade after desktop unregisters');
        external.stdin.end();
        requireCleanExit(await deadline(externalExit, 20000, 'external fixture shutdown'), 'external Rust');
      }
      for (const port of ports) assert(!(await listening(port)), `owned listener still open: ${port}`);
      results.push({ adopted, exit: status, ports, catalogSize, detachedCatalogSize, desktopConfig: true, listenersClosed: true });
    } catch (error) {
      throw new Error(`${adopted ? 'adopted' : 'owned'} Electron smoke failed: ${error.message.replaceAll(token, '[fixture-token]')}\n${diagnostics.replaceAll(token, '[fixture-token]')}`);
    } finally {
      // Failure cleanup never becomes passing exit or joined-shutdown evidence.
      if (child && child.exitCode === null && child.signalCode === null) {
        child.kill('SIGKILL');
        await deadline(exited, 5000, 'failed Electron cleanup').catch(() => {});
      }
      if (app) await deadline(app.close(), 5000, 'Playwright cleanup').catch(() => {});
      if (external && external.exitCode === null && external.signalCode === null) {
        external.stdin?.end();
        await deadline(externalExit, 5000, 'cleanup external').catch(async () => {
          external.kill('SIGKILL');
          await deadline(externalExit, 5000, 'failed external cleanup').catch(() => {});
        });
      }
      fs.rmSync(root, { recursive: true, force: true });
    }
  }
  const report = { platform: 'linux', commit: process.env.GITHUB_SHA ?? null, executable, scope: 'packaged Electron ownership, no visual/model claim', results };
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [executable, output] = process.argv.slice(2);
  if (!executable || !output) { console.error('Usage: smoke-electron-ownership.mjs /absolute/unpacked/Workspacer /absolute/report.json'); process.exitCode = 1; }
  else await smoke(executable, output);
}
