/**
 * Main-process client for the hub event bus. One WebSocket to the hub does two
 * jobs:
 *
 *   1. Subscribes to bus events and forwards them to the renderer (hub:event).
 *   2. Registers main as a capability *provider* — it answers `call` frames for
 *      methods like `agents.list` / `agents.sendMessage`. This is the inverse of
 *      events (request/reply) and the same path the MCP facade will drive.
 *
 * Reconnects with backoff; re-subscribes and re-registers on every (re)connect.
 */

import WebSocket from 'ws';
import { BrowserWindow } from 'electron';
import { HUB_BUS_URL, getHubToken } from './hubDaemon';

const TOPICS = ['*'];

type CapabilityHandler = (params: unknown) => Promise<unknown> | unknown;

interface HubEvent {
  id: string;
  type: string;
  source: string;
  time: string;
  data?: unknown;
}

const handlers = new Map<string, CapabilityHandler>();
let ws: WebSocket | null = null;
let mainWindow: BrowserWindow | null = null;
let stopped = false;
let backoff = 200;
let connected = false;

// Outbound calls: main as a *caller* (the inverse of the provider role above).
// Lets the renderer reach hub-owned capabilities — e.g. the shared layout
// document — through main, mirroring how the web build calls the bus directly.
const CALL_TIMEOUT_MS = 15000;
let callSeq = 0;
const pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: NodeJS.Timeout }>();

/** Invoke a capability on the bus and resolve with its result. Rejects if the
 *  socket is down, the call errors, or it times out. Our ids are prefixed `m`
 *  so they never collide with the hub-assigned numeric ids of inbound calls. */
export function callHub<T = unknown>(method: string, params: unknown = {}): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      reject(new Error('hub not connected'));
      return;
    }
    const id = 'm' + (++callSeq);
    const timer = setTimeout(() => {
      if (pending.has(id)) { pending.delete(id); reject(new Error(`hub call timeout: ${method}`)); }
    }, CALL_TIMEOUT_MS);
    pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer });
    send({ op: 'call', id, method, params });
  });
}

/** Current bus connection state — lets a late-mounting renderer sync up. */
export function isHubConnected(): boolean {
  return connected;
}

export function setHubMainWindow(win: BrowserWindow): void {
  mainWindow = win;
}

/** Register a capability main provides on the bus. Call before startHubClient. */
export function registerCapability(method: string, handler: CapabilityHandler): void {
  handlers.set(method, handler);
}

function forward(channel: string, ...args: unknown[]): void {
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send(channel, ...args);
  }
}

function send(frame: Record<string, unknown>): void {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(frame));
  }
}

function handleCall(id: string, method: string, params: unknown): void {
  const handler = handlers.get(method);
  if (!handler) {
    send({ op: 'error', id, error: `no handler for ${method}` });
    return;
  }
  Promise.resolve()
    .then(() => handler(params))
    .then(
      (result) => send({ op: 'result', id, result: result ?? null }),
      (err) => send({ op: 'error', id, error: err?.message ? String(err.message) : String(err) }),
    );
}

function connect(): void {
  if (stopped) return;
  // Remove listeners from the old socket before creating a new one so they
  // don't accumulate across reconnects.
  if (ws) {
    ws.removeAllListeners();
  }
  // When remote auth is on the hub rejects /bus without the token; the local
  // client presents it too. No token configured → URL is unchanged.
  const token = getHubToken();
  const url = token ? `${HUB_BUS_URL}?token=${encodeURIComponent(token)}` : HUB_BUS_URL;
  ws = new WebSocket(url);

  ws.on('open', () => {
    backoff = 200;
    connected = true;
    send({ op: 'subscribe', topics: TOPICS });
    if (handlers.size > 0) {
      send({ op: 'register', methods: Array.from(handlers.keys()) });
    }
    forward('hub:status', { connected: true });
    console.log(`[hub-client] connected; subscribed ${TOPICS.join(',')}; provides ${Array.from(handlers.keys()).join(',') || '(none)'}`);
  });

  ws.on('message', (raw: WebSocket.RawData) => {
    let frame: { op?: string; event?: HubEvent; id?: string; method?: string; params?: unknown; result?: unknown; error?: string };
    try {
      frame = JSON.parse(raw.toString());
    } catch {
      return;
    }
    switch (frame.op) {
      case 'event':
        // High-frequency PTY mirror events are for remote clients only; the
        // local renderer draws the terminal straight from claudemon, so don't
        // echo them back to it (we'd receive our own publishes via the '*' sub).
        if (frame.event && !frame.event.type.startsWith('pty.')) {
          forward('hub:event', frame.event);
          // The shared layout document changed somewhere (this desktop, the web
          // remote, or another client) — push it to the renderer on its own
          // channel so the layout reconciles without filtering the event firehose.
          if (frame.event.type === 'layout.changed') {
            forward('layout:changed', frame.event.data);
          }
        }
        break;
      case 'call':
        if (frame.id && frame.method) handleCall(frame.id, frame.method, frame.params);
        break;
      case 'result': {
        const c = frame.id ? pending.get(frame.id) : undefined;
        if (c && frame.id) { pending.delete(frame.id); clearTimeout(c.timer); c.resolve(frame.result); }
        break;
      }
      case 'error': {
        const c = frame.id ? pending.get(frame.id) : undefined;
        if (c && frame.id) { pending.delete(frame.id); clearTimeout(c.timer); c.reject(new Error(frame.error || 'hub error')); }
        break;
      }
      // hello / subscribed / registered acks: nothing to do.
    }
  });

  ws.on('close', () => {
    connected = false;
    // Fail any in-flight outbound calls — their socket is gone.
    for (const [id, c] of pending) { clearTimeout(c.timer); c.reject(new Error('hub disconnected')); pending.delete(id); }
    forward('hub:status', { connected: false });
    scheduleReconnect();
  });

  ws.on('error', () => {
    try { ws?.close(); } catch { /* noop */ }
  });
}

function scheduleReconnect(): void {
  if (stopped) return;
  const wait = backoff;
  backoff = Math.min(backoff * 2, 5000);
  setTimeout(connect, wait);
}

/** Publish an event onto the bus (e.g. from a renderer-triggered plugin hotkey). */
export function publishToHub(ev: { type: string; source?: string; data?: unknown }): void {
  send({ op: 'publish', event: { source: 'workspacer', ...ev } });
}

export function startHubClient(): void {
  // Idempotent: if ws is already live, skip re-starting.
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return;
  stopped = false;
  backoff = 200; // reset backoff on each explicit start
  connect();
}

export function stopHubClient(): void {
  stopped = true;
  try { ws?.close(); } catch { /* noop */ }
  ws = null;
  // Drain all in-flight outbound calls so their timers don't leak.
  for (const [id, c] of pending) {
    clearTimeout(c.timer);
    c.reject(new Error('hub stopped'));
    pending.delete(id);
  }
}
