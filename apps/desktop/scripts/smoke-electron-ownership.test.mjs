import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { EventEmitter } from 'node:events';
import { requireCleanExit, exitReceipt, deadline, isolatedEnvironment, checkModernCatalog, checkFacadeHealth, checkDetachedCatalog, checkHelpResult, detachedFacade } from './smoke-electron-ownership.mjs';

test('only an actual unsignalled zero exit qualifies as clean ownership', async () => {
  requireCleanExit({ code: 0, signal: null }, 'fixture');
  for (const result of [{ code: null, signal: 'SIGKILL' }, { code: 1, signal: null }, { code: 0, signal: 'SIGTERM' }])
    assert.throws(() => requireCleanExit(result, 'fixture'));
  const child = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
  const receipt = exitReceipt(child);
  child.emit('exit', 0, null);
  assert.deepEqual(await receipt, { code: 0, signal: null });
  assert.deepEqual(await exitReceipt(Object.assign(new EventEmitter(), { exitCode: 0, signalCode: null })), { code: 0, signal: null });
});
test('timeout refuses an unconfirmed shutdown instead of manufacturing success', async () => {
  await assert.rejects(deadline(new Promise(() => {}), 5, 'join'), /join timed out/);
});
test('isolated process environment excludes inherited credentials and bootstrap overrides', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'electron-env-'));
  try {
    const env = isolatedEnvironment(root, 10000, { PATH: '/bin', DISPLAY: ':99', HUB_TOKEN: 'secret',
      WORKSPACER_RUST_HUB_BINARY: '/foreign', ELECTRON_RUN_AS_NODE: '1', NODE_OPTIONS: '--require evil', HOME: '/real' });
    assert.equal(env.DISPLAY, ':99'); assert.equal(env.WORKSPACER_PORT_OFFSET, '10000');
    for (const key of ['HUB_TOKEN', 'NODE_OPTIONS', 'WORKSPACER_RUST_HUB_BINARY', 'ELECTRON_RUN_AS_NODE']) assert.equal(env[key], undefined);
    assert(env.HOME.startsWith(root)); assert(fs.statSync(env.HOME).isDirectory());
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
test('modern catalog uses captured operator floor and refuses incomplete/duplicate metadata', () => {
  const tools = JSON.parse(fs.readFileSync(new URL('../../../services/hub-rs/assets/mcp-effective-tools.json', import.meta.url), 'utf8')).operator;
  assert.equal(tools.length, 100, 'review the retained operator floor when the core catalog changes');
  const valid = { jsonrpc: '2.0', id: 1, result: { resultType: 'complete', ttlMs: 0, cacheScope: 'private', tools } };
  assert.equal(checkModernCatalog(valid), 100);
  for (const patch of [{ ttlMs: undefined }, { cacheScope: 'public' }, { tools: [] },
    { tools: tools.slice(1) }, { tools: [...tools, tools[0]] }, { resultType: undefined }])
    assert.throws(() => checkModernCatalog({ ...valid, result: { ...valid.result, ...patch } }));
});
test('MCP health requires the actual Rust identity and exact owning endpoints', () => {
  const expected = { status: 'ok', service: 'workspacer-mcp-facade', implementation: 'rust',
    hubConnected: true, pluginCatalogReady: true, listenAddr: '127.0.0.1:1234', hubUrl: 'ws://127.0.0.1:5678/bus' };
  checkFacadeHealth(expected, 5678, 1234);
  for (const key of Object.keys(expected)) {
    const wrong = { ...expected }; delete wrong[key];
    assert.throws(() => checkFacadeHealth(wrong, 5678, 1234));
  }
  assert.throws(() => checkFacadeHealth(expected, 5679, 1234));
  assert.throws(() => checkFacadeHealth(expected, 5678, 1235));
});

test('detached facade retains modern metadata but removes desktop-owned capability', () => {
  const valid = { jsonrpc: '2.0', id: 1, result: { resultType: 'complete', ttlMs: 0, cacheScope: 'private', tools: [{ name: 'help' }] } };
  assert.equal(checkDetachedCatalog(valid), 1);
  assert.throws(() => checkModernCatalog(valid), /100-tool/);
  for (const patch of [{ ttlMs: undefined }, { cacheScope: 'public' }, { resultType: undefined },
    { tools: [] }, { tools: [{ name: 'help' }, { name: 'help' }] },
    { tools: [{ name: 'help' }, { name: 'get_host_cwd' }] }])
    assert.throws(() => checkDetachedCatalog({ ...valid, result: { ...valid.result, ...patch } }));
  const help = { jsonrpc: '2.0', id: 1, result: { content: [{ type: 'text', text: 'Available tools' }] } };
  checkHelpResult(help);
  for (const result of [{ isError: true, content: help.result.content }, { content: [] }, { content: [{ type: 'text', text: ' ' }] }])
    assert.throws(() => checkHelpResult({ ...help, result }));
  assert.throws(() => checkHelpResult({ ...help, error: { code: -32603 } }));
});
test('detached probe actually calls retained help after fresh modern tools/list', async () => {
  const seen = [];
  let rejectHelp = false;
  const server = http.createServer(async (req, res) => {
    let raw = ''; for await (const chunk of req) raw += chunk;
    const body = JSON.parse(raw); seen.push(body.method);
    assert.equal(req.headers.authorization, 'Bearer fixture');
    assert.equal(req.headers['mcp-method'], body.method);
    assert.equal(body.params._meta['io.modelcontextprotocol/protocolVersion'], '2026-07-28');
    if (body.method === 'tools/call') {
      assert.equal(body.params.name, 'help'); assert.deepEqual(body.params.arguments, {});
      assert.equal(req.headers['mcp-name'], 'help');
    }
    const result = body.method === 'tools/list'
      ? { resultType: 'complete', ttlMs: 0, cacheScope: 'private', tools: [{ name: 'help' }] }
      : { isError: rejectHelp, content: [{ type: 'text', text: 'Retained local help' }] };
    res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ jsonrpc: '2.0', id: 1, result }));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    assert.equal(await detachedFacade(server.address().port, 'fixture'), 1);
    assert.deepEqual(seen, ['tools/list', 'tools/call']);
    rejectHelp = true;
    await assert.rejects(detachedFacade(server.address().port, 'fixture'), /help call failed/);
  } finally { await new Promise((resolve) => server.close(resolve)); }
});
