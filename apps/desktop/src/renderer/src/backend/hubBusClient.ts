/**
 * Minimal hub-bus WebSocket client for the *web* build of the renderer.
 *
 * It speaks the same `{op}` frame protocol as `services/hub/cmd/hub/remote.html`
 * and the Go bus (`services/hub/internal/bus/bus.go`): RPC via `call`/`result`/`error`, pub/sub via
 * `subscribe`/`unsubscribe`/`event`. In the Electron build the main process owns
 * an equivalent client (`src/main/services/hubClient.ts`) and the renderer never
 * touches the socket directly — this module is the browser-side replacement that
 * lets the same React app run against the hub when there is no Electron preload.
 */

export interface HubEventEnvelope {
  id: string;
  type: string;
  source: string;
  time: string;
  data?: unknown;
}

type EventHandler = (ev: HubEventEnvelope) => void;
type StatusHandler = (connected: boolean) => void;

interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

const CALL_TIMEOUT_MS = 15000;
// A socket that has seen no inbound frame for this long when the page returns to
// the foreground is treated as a zombie (browsers suspend background sockets
// without always firing onclose) and replaced with a fresh connection.
const STALE_MS = 30000;

export class HubBusClient {
  private ws: WebSocket | null = null;
  private connected = false;
  private callSeq = 0;
  private readonly calls = new Map<string, PendingCall>();
  /** topic-prefix → set of handlers. Matched against each event's type. */
  private readonly subscriptions = new Map<string, Set<EventHandler>>();
  private readonly statusHandlers = new Set<StatusHandler>();
  /** Call frames produced before the socket opened, flushed on connect. */
  private readonly sendQueue: string[] = [];
  private backoff = 500;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private closedByUser = false;
  /** Set once the server closed us with an auth-rejection code (1008/4401).
   *  A bad/expired token won't succeed on retry, so wake() must not reopen. */
  private authRejected = false;
  /** True once any connection has succeeded — distinguishes the first connect
   *  from a reconnect so reconnect handlers don't fire on initial mount. */
  private hasConnectedOnce = false;
  /** Wall-clock ms of the last inbound frame; drives the staleness check. */
  private lastActivity = 0;
  private readonly reconnectHandlers = new Set<() => void>();

  /**
   * @param token  bearer secret for the `/bus` gate.
   * @param baseUrl  full `ws[s]://host:port/bus` URL to connect to. Omitted in
   *   the web build, where the renderer is served by the hub itself and the URL
   *   is derived from `location`. The Electron build passes the local hub's URL
   *   explicitly, since there the renderer isn't served off the bus host.
   */
  constructor(
    private readonly token: string,
    private readonly baseUrl?: string,
  ) {}

  /** Bound so it can be added/removed as a DOM listener. The browser throttles a
   *  backgrounded tab's reconnect timer and suspends its socket, so we also
   *  reconnect proactively the moment the page is shown again or the net is back. */
  private readonly onWake = (): void => {
    this.wake();
  };

  // ── connection lifecycle ──────────────────────────────────────────────

  start(): void {
    this.closedByUser = false;
    this.authRejected = false;
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', this.onWake);
    if (typeof window !== 'undefined') window.addEventListener('online', this.onWake);
    this.open();
  }

  /**
   * Force an immediate reconnect when the page returns to the foreground or the
   * network comes back, unless the current socket is provably live. Without this
   * a tab left in the background sits on a dead/suspended socket — and a stale
   * UI — until the user manually refreshes.
   */
  private wake(): void {
    if (this.closedByUser || this.authRejected) return;
    // Only act when the page is actually visible (visibilitychange also fires on hide).
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
    const live =
      this.ws?.readyState === WebSocket.OPEN && Date.now() - this.lastActivity < STALE_MS;
    if (live) return;
    this.backoff = 500;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.detachSocket();
    this.open();
  }

  /**
   * Detach handlers from the current socket and close it, so a zombie socket's
   * late onclose/onerror can't perturb the fresh connection wake() opens next.
   */
  private detachSocket(): void {
    const ws = this.ws;
    this.ws = null;
    if (!ws) return;
    ws.onopen = null;
    ws.onmessage = null;
    ws.onerror = null;
    ws.onclose = null;
    try {
      ws.close();
    } catch {
      /* ignore */
    }
    this.setConnected(false);
  }

  /**
   * Fires after every successful *reconnect* (never the first connect). The bus
   * re-asserts topic subscriptions on reconnect but does not replay the snapshots
   * callers fetched once at mount, so listeners use this to re-sync that state.
   */
  onReconnect(handler: () => void): () => void {
    this.reconnectHandlers.add(handler);
    return () => this.reconnectHandlers.delete(handler);
  }

  private wsURL(): string {
    const base =
      this.baseUrl ?? `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/bus`;
    const sep = base.includes('?') ? '&' : '?';
    return `${base}${sep}token=${encodeURIComponent(this.token)}`;
  }

  private open(): void {
    try {
      this.ws = new WebSocket(this.wsURL());
    } catch {
      this.scheduleReconnect();
      return;
    }

    this.ws.onopen = () => {
      this.backoff = 500;
      this.lastActivity = Date.now();
      this.setConnected(true);
      // Re-assert every active topic subscription after a (re)connect.
      for (const topic of this.subscriptions.keys()) this.sendSubscribe(topic, true);
      // Flush calls that were queued before the socket finished connecting
      // (e.g. the renderer's initial getAllClaudeSessions / getConfig at mount).
      const queued = this.sendQueue.splice(0);
      for (const frame of queued) this.ws?.send(frame);
      // Let callers re-sync after a reconnect (skipped on the very first connect,
      // where the mount-time fetches already loaded current state).
      if (this.hasConnectedOnce) {
        for (const h of this.reconnectHandlers) h();
      }
      this.hasConnectedOnce = true;
    };

    this.ws.onmessage = (ev) => this.onFrame(ev.data);

    this.ws.onclose = (ev) => {
      this.setConnected(false);
      // 1008 / 4401 = auth rejected — no point reconnecting with a bad token.
      if (ev.code === 1008 || ev.code === 4401) {
        this.authRejected = true;
        return;
      }
      if (!this.closedByUser) this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      try {
        this.ws?.close();
      } catch {
        /* ignore */
      }
    };
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer || this.closedByUser) return;
    const delay = this.backoff;
    this.backoff = Math.min(this.backoff * 2, 10000);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.open();
    }, delay);
  }

  stop(): void {
    this.closedByUser = true;
    if (typeof document !== 'undefined')
      document.removeEventListener('visibilitychange', this.onWake);
    if (typeof window !== 'undefined') window.removeEventListener('online', this.onWake);
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    try {
      this.ws?.close();
    } catch {
      /* ignore */
    }
    this.ws = null;
  }

  private setConnected(v: boolean): void {
    if (this.connected === v) return;
    this.connected = v;
    for (const h of this.statusHandlers) h(v);
  }

  isConnected(): boolean {
    return this.connected;
  }

  onStatus(handler: StatusHandler): () => void {
    this.statusHandlers.add(handler);
    handler(this.connected);
    return () => this.statusHandlers.delete(handler);
  }

  // ── frames ────────────────────────────────────────────────────────────

  private onFrame(raw: unknown): void {
    // Any inbound frame proves the socket is alive — feeds the staleness check.
    this.lastActivity = Date.now();
    let f: { op?: string; id?: string; result?: unknown; error?: string; event?: HubEventEnvelope };
    try {
      f = JSON.parse(typeof raw === 'string' ? raw : '');
    } catch {
      return;
    }
    if (f.op === 'result') {
      const c = f.id ? this.calls.get(f.id) : undefined;
      if (c && f.id) {
        this.calls.delete(f.id);
        clearTimeout(c.timer);
        c.resolve(f.result);
      }
    } else if (f.op === 'error') {
      const c = f.id ? this.calls.get(f.id) : undefined;
      if (c && f.id) {
        this.calls.delete(f.id);
        clearTimeout(c.timer);
        c.reject(new Error(f.error || 'hub error'));
      }
    } else if (f.op === 'event' && f.event) {
      this.dispatchEvent(f.event);
    }
  }

  private dispatchEvent(ev: HubEventEnvelope): void {
    for (const [topic, handlers] of this.subscriptions) {
      if (topicMatches(topic, ev.type)) {
        for (const h of handlers) h(ev);
      }
    }
  }

  // ── RPC ───────────────────────────────────────────────────────────────

  call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const id = 'c' + ++this.callSeq;
      const timer = setTimeout(() => {
        if (this.calls.has(id)) {
          this.calls.delete(id);
          reject(new Error(`hub call timeout: ${method}`));
        }
      }, CALL_TIMEOUT_MS);
      this.calls.set(id, { resolve: resolve as (v: unknown) => void, reject, timer });
      const frame = JSON.stringify({ op: 'call', id, method, params });
      // Send now if connected, else queue until onopen. The pending call's
      // timeout still applies, so a never-connecting socket rejects cleanly.
      if (this.ws && this.ws.readyState === WebSocket.OPEN) {
        this.ws.send(frame);
      } else {
        this.sendQueue.push(frame);
      }
    });
  }

  // ── pub/sub ───────────────────────────────────────────────────────────

  /**
   * Subscribe to a hub topic (may be a wildcard prefix like `workflow.*`).
   * Returns an unsubscribe function. Reference-counted per topic so multiple
   * callers can share one server-side subscription.
   */
  subscribe(topic: string, handler: EventHandler): () => void {
    let set = this.subscriptions.get(topic);
    if (!set) {
      set = new Set();
      this.subscriptions.set(topic, set);
      this.sendSubscribe(topic, true);
    }
    set.add(handler);
    return () => {
      const s = this.subscriptions.get(topic);
      if (!s) return;
      s.delete(handler);
      if (s.size === 0) {
        this.subscriptions.delete(topic);
        this.sendSubscribe(topic, false);
      }
    };
  }

  private sendSubscribe(topic: string, on: boolean): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    this.ws.send(JSON.stringify({ op: on ? 'subscribe' : 'unsubscribe', topics: [topic] }));
  }
}

/** `workflow.*` matches `workflow.started`; exact topics match exactly. */
function topicMatches(topic: string, eventType: string): boolean {
  if (topic === eventType) return true;
  if (topic.endsWith('.*')) return eventType.startsWith(topic.slice(0, -1));
  if (topic === '*') return true;
  return false;
}
