#!/usr/bin/env node
// Workspacer plugin sidecar scaffold — ZERO dependencies.
//
// Node >= 22 (global WebSocket) and >= 18 (global fetch). Uses only Node
// built-ins: http, fs, path, child_process (+ the global WebSocket/fetch).
//
// It reads its own plugin.json for the bus topics to subscribe to (`consumes`)
// and the capabilities it may call (`capabilities`), connects to the hub bus,
// keeps reconnecting if the hub is down, and serves a tiny status pane + a
// /health endpoint. Put your logic in onEvent().
//
// See the skill's SKILL.md and ../workspacer-plugins/test-on-save/server.js for
// a full real example (local fs.watch + agents.sendMessage + notifications.post).

'use strict';
const http = require('http');
const fs = require('fs');
const path = require('path');
// const { exec } = require('child_process'); // uncomment if you run commands

const DIR = __dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(DIR, 'plugin.json'), 'utf8'));
const PORT = Number(process.env.PORT || (manifest.server && manifest.server.port) || 9200);

// The hub launches the sidecar with the bus token in HUB_TOKEN. We also accept
// WKS_BUS_TOKEN / a .bus-token file so the scaffold runs however you wire it in
// dev. The hub does not inject the bus URL, so we default it (override with
// WKS_BUS_URL / HUB_BUS_URL for non-standard setups).
const BUS_URL = process.env.WKS_BUS_URL || process.env.HUB_BUS_URL || 'ws://127.0.0.1:7895/bus';
function readToken() {
  if (process.env.HUB_TOKEN) return process.env.HUB_TOKEN;
  if (process.env.WKS_BUS_TOKEN) return process.env.WKS_BUS_TOKEN;
  try { return fs.readFileSync(path.join(DIR, '.bus-token'), 'utf8').trim(); } catch { return ''; }
}

// Host-injected settings (from manifest `settings`), passed as JSON in env.
// Read ONLY at spawn — a settings change restarts the sidecar with fresh values.
let settings = {};
try { settings = JSON.parse(process.env.WKS_SETTINGS || '{}'); } catch {}

const TOPICS = manifest.consumes || [];
const recent = [];
let ws = null, connected = false, callSeq = 0;
const pending = new Map();

function log(msg) {
  console.log('[' + manifest.id + '] ' + msg);
  recent.unshift(new Date().toISOString() + '  ' + msg);
  if (recent.length > 100) recent.pop();
}

// Call a hub capability (must be declared in plugin.json `capabilities`).
// Undeclared calls come back as an error frame → this promise rejects.
function call(method, params) {
  return new Promise((resolve, reject) => {
    if (!connected) return reject(new Error('not connected'));
    const id = 'c' + (++callSeq);
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ op: 'call', id, method, params: params || {} }));
    setTimeout(() => { if (pending.has(id)) { pending.delete(id); reject(new Error('timeout')); } }, 8000);
  });
}

// Publish an event (must be declared in `emits`, else it is dropped silently).
function publish(type, data) {
  if (connected) ws.send(JSON.stringify({ op: 'publish', event: { type, source: manifest.id, data: data || {} } }));
}

function connect() {
  const tok = readToken();
  ws = new WebSocket(BUS_URL + (tok ? '?token=' + encodeURIComponent(tok) : ''));
  ws.addEventListener('open', () => {
    connected = true;
    if (TOPICS.length) ws.send(JSON.stringify({ op: 'subscribe', topics: TOPICS }));
    log('connected; subscribed to ' + (TOPICS.join(', ') || '(nothing)'));
  });
  ws.addEventListener('message', (ev) => {
    let f; try { f = JSON.parse(ev.data); } catch { return; }
    if (f.op === 'event' && f.event) {
      Promise.resolve(onEvent(f.event)).catch((e) => log('onEvent error: ' + e.message));
    } else if (f.op === 'result' && pending.has(f.id)) {
      pending.get(f.id).resolve(f.result); pending.delete(f.id);
    } else if (f.op === 'error' && pending.has(f.id)) {
      pending.get(f.id).reject(new Error(f.error)); pending.delete(f.id);
    }
  });
  // Reconnect loop: if the hub is down or drops us, retry — never crash.
  ws.addEventListener('close', () => { connected = false; setTimeout(connect, 1500); });
  ws.addEventListener('error', () => { try { ws.close(); } catch {} });
}

// ── Your logic ──────────────────────────────────────────────────────────────
// Handle an inbound bus event. Only event types in `consumes` are delivered.
//
// NOTE ON FILESYSTEM WATCHING: the fs.watch *capability* is pane-scoped — its
// ${agentCwd} grant only resolves for webview pane tokens, so a sidecar can't
// use it. Watch the tree LOCALLY with node's fs.watch instead (the sidecar runs
// on the same host as the workspace). See the test-on-save example.
async function onEvent(event) {
  const t = event.type;
  const d = event.data || {};
  if (t === 'agent.state_changed') {
    // { sessionId, hookEvent, mode, cwd }
    log('agent ' + d.sessionId + ' → ' + d.mode + (d.cwd ? ' @ ' + d.cwd : ''));
    return;
  }
  if (t === 'agent.snapshot') {
    // per-session snapshot
    return;
  }
}

// ── Status pane + health ──────────────────────────────────────────────────────
const server = http.createServer((req, res) => {
  if (req.url === (manifest.server && manifest.server.health || '/health')) {
    res.writeHead(200); return res.end('ok');
  }
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end('<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=2>'
    + '<title>' + escapeHtml(manifest.name) + '</title>'
    + '<body style="font-family:system-ui;background:var(--wks-bg-base,#161616);'
    + 'color:var(--wks-text-primary,#e8e8e8);margin:0;padding:14px">'
    + '<h2 style="font-size:1rem">' + escapeHtml(manifest.name) + '</h2>'
    + '<p style="color:var(--wks-text-muted,#888);font-size:.8rem">'
    + (connected ? '\u{1F7E2} connected to hub' : '\u{1F534} disconnected')
    + ' · subscribed to ' + escapeHtml(TOPICS.join(', ') || '(nothing)') + '</p>'
    + '<pre style="font-size:.7rem;color:var(--wks-text-faint,#777);white-space:pre-wrap">'
    + (recent.map(escapeHtml).join('\n') || 'waiting for events…') + '</pre>');
});
function escapeHtml(s) { return String(s).replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c])); }

server.listen(PORT, '127.0.0.1', () => log('pane on http://127.0.0.1:' + PORT));
connect();
