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
import { hubBusUrl, getHubToken } from './hubDaemon';
import { notifySystem } from './systemNotice';

/**
 * Methods whose withholding is a real LOSS rather than a handover between two
 * equivalent implementations. Kept in agreement with the `degraded` entries of
 * services/hub/cmd/brain/delegation_guard_test.go's declaredOverlap.
 */
const DEGRADED_WHEN_WITHHELD = new Set([
  'notifications.post',
  'analytics.summary',
  'analytics.recent',
]);

/** One notice per process, not one per reconnect. */
let warnedDegraded = false;
import { IPC } from '../shared/ipcChannels';

const TOPICS = ['*'];

type CapabilityHandler = (params: unknown) => Promise<unknown> | unknown;

export interface HubEvent {
  id: string;
  type: string;
  source: string;
  /** Federation: the peer hub this event was republished from (stamped by the
   *  local hub's federation link). Absent/empty = a local event. */
  hub?: string;
  time: string;
  data?: unknown;
}

const handlers = new Map<string, CapabilityHandler>();

// Main-process event subscribers (federationBridge etc). The renderer gets its
// copy over hub:event; these listeners are how MAIN reacts to bus events.
const eventListeners = new Set<(ev: HubEvent) => void>();

/** Subscribe main-process code to bus events (every non-pty.* event, same gate
 *  as the renderer forward). Returns an unsubscribe function. */
export function subscribeHubEvents(listener: (ev: HubEvent) => void): () => void {
  eventListeners.add(listener);
  return () => {
    eventListeners.delete(listener);
  };
}

// Main-process connect listeners: fired on every successful (re)connect, after
// the subscribe/register frames go out. federationBridge uses this to ask the
// hub for already-connected peers — peer lifecycle events fire on transitions
// only, and a hub restart (e.g. saving peers.json) drops this socket, so the
// peer's connected event usually fires before we're back to hear it.
const connectListeners = new Set<() => void>();

/** Subscribe main-process code to bus (re)connects. Returns an unsubscribe. */
export function subscribeHubConnected(listener: () => void): () => void {
  connectListeners.add(listener);
  return () => {
    connectListeners.delete(listener);
  };
}

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
const pending = new Map<
  string,
  { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: NodeJS.Timeout }
>();

/** Invoke a capability on the bus and resolve with its result. Rejects if the
 *  socket is down, the call errors, or it times out. Our ids are prefixed `m`
 *  so they never collide with the hub-assigned numeric ids of inbound calls. */
export function callHub<T = unknown>(method: string, params: unknown = {}): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      reject(new Error('hub not connected'));
      return;
    }
    const id = 'm' + ++callSeq;
    const timer = setTimeout(() => {
      if (pending.has(id)) {
        pending.delete(id);
        reject(new Error(`hub call timeout: ${method}`));
      }
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

/** Push an event to the renderer window (main → renderer). No-op if the window
 *  is gone. Used by capabilities that must reach the UI — e.g. terminals.open
 *  asking the renderer to open a visible terminal pane. */
export function emitToRenderer(channel: string, ...args: unknown[]): void {
  forward(channel, ...args);
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
  const base = hubBusUrl();
  const url = token ? `${base}?token=${encodeURIComponent(token)}` : base;
  ws = new WebSocket(url);

  ws.on('open', () => {
    backoff = 200;
    connected = true;
    send({ op: 'subscribe', topics: TOPICS });
    if (handlers.size > 0) {
      send({ op: 'register', methods: Array.from(handlers.keys()) });
    }
    forward('hub:status', { connected: true });
    console.log(
      `[hub-client] connected; subscribed ${TOPICS.join(',')}; provides ${Array.from(handlers.keys()).join(',') || '(none)'}`,
    );
    // A listener throwing must not kill the open handler (same rule as the
    // event listeners below).
    for (const listener of connectListeners) {
      try {
        listener();
      } catch (err) {
        console.error('[hub-client] connect listener failed:', err);
      }
    }
  });

  ws.on('message', (raw: WebSocket.RawData) => {
    let frame: {
      op?: string;
      event?: HubEvent;
      id?: string;
      method?: string;
      methods?: string[];
      params?: unknown;
      result?: unknown;
      error?: string;
    };
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
          // Plugin settings changed somewhere (this desktop, web, or remote).
          // Bridge it to the renderer's plugin-settings channel so open plugin
          // panes re-apply and the Settings UI updates live regardless of which
          // client made the edit. Local writes also get an immediate push from
          // the SET handler; a duplicate here just re-injects the same values.
          if (frame.event.type === 'plugin.settings.changed') {
            const d = frame.event.data as
              { id?: string; values?: Record<string, unknown> } | undefined;
            if (d?.id) forward(IPC.HUB_PLUGIN_SETTINGS_CHANGED, d.id, d.values ?? {});
          }
          // Main's own subscribers (federationBridge ingests hub-stamped
          // agent.* events and hub.peer.* lifecycle here). A listener throwing
          // must not kill the socket's message handler.
          for (const listener of eventListeners) {
            try {
              listener(frame.event);
            } catch (err) {
              console.error('[hub-client] event listener failed:', err);
            }
          }
        }
        break;
      case 'call':
        if (frame.id && frame.method) handleCall(frame.id, frame.method, frame.params);
        break;
      case 'result': {
        const c = frame.id ? pending.get(frame.id) : undefined;
        if (c && frame.id) {
          pending.delete(frame.id);
          clearTimeout(c.timer);
          c.resolve(frame.result);
        }
        break;
      }
      case 'error': {
        const c = frame.id ? pending.get(frame.id) : undefined;
        if (c && frame.id) {
          pending.delete(frame.id);
          clearTimeout(c.timer);
          c.reject(new Error(frame.error || 'hub error'));
        }
        break;
      }
      case 'registered': {
        // The hub's router is first-registration-wins (a capability-hijack
        // guard — services/hub internal/bus/rpc.go): a method already owned by
        // another LIVE connection, or answered by a hub-local handler, is
        // withheld from us and the ack lists only what actually registered.
        // That is what makes registering our full surface against an ADOPTED
        // `workspacer serve` hub safe — but it is NOT free: for three methods
        // the adopted full-scope brain is a degraded stand-in (see the
        // adopted-hub note in hubCapabilities.ts), and this ack is the only
        // moment either process can notice. A console.log has no consumer in a
        // packaged app, so say it at warn level and raise ONE system notice for
        // the degraded set.
        const requested = Array.from(handlers.keys());
        const accepted = new Set(frame.methods ?? []);
        const withheld = requested.filter((m) => !accepted.has(m));
        if (withheld.length > 0) {
          console.warn(
            `[hub-client] ${withheld.length}/${requested.length} capability method(s) withheld — ` +
              `already provided by another connection (adopted full-scope brain?): ${withheld.join(', ')}`,
          );
          const degraded = withheld.filter((m) => DEGRADED_WHEN_WITHHELD.has(m));
          if (degraded.length > 0 && !warnedDegraded) {
            warnedDegraded = true;
            notifySystem({
              level: 'warn',
              key: 'hub-caps-withheld',
              title: 'Some capabilities are served by the external server',
              detail:
                `This app is attached to an external Workspacer server, which answers ` +
                `${degraded.join(', ')} itself. Those are degraded there: ` +
                `desktop notifications are only logged and usage analytics report zeros.`,
            });
          }
        }
        break;
      }
      // hello / subscribed acks: nothing to do.
    }
  });

  ws.on('close', () => {
    connected = false;
    // Fail any in-flight outbound calls — their socket is gone.
    for (const [id, c] of pending) {
      clearTimeout(c.timer);
      c.reject(new Error('hub disconnected'));
      pending.delete(id);
    }
    forward('hub:status', { connected: false });
    scheduleReconnect();
  });

  ws.on('error', () => {
    try {
      ws?.close();
    } catch {
      /* noop */
    }
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
  try {
    ws?.close();
  } catch {
    /* noop */
  }
  ws = null;
  // Drain all in-flight outbound calls so their timers don't leak.
  for (const [id, c] of pending) {
    clearTimeout(c.timer);
    c.reject(new Error('hub stopped'));
    pending.delete(id);
  }
}
