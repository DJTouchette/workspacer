import WebSocket from 'ws';
import { createHash } from 'crypto';
import { getPairedWorkerTarget } from './remoteServer';

/** One outbound, host-only socket. Credentials never enter the renderer or an agent. */
export class PairedWorkerConnection {
  private socket?: WebSocket;
  private stopped = false;
  private identity = '';
  private connecting?: Promise<void>;
  private sequence = 0;
  private pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: NodeJS.Timeout }>();
  private retry?: NodeJS.Timeout;
  onEvent: (event: { type: string; data?: unknown }) => void = () => {};
  onConnected: () => void = () => {};
  onDisconnected: () => void = () => {};

  constructor(private setting = getPairedWorkerTarget) {}

  stop(): void { this.stopped = true; if (this.retry) clearTimeout(this.retry); this.retry = undefined; const socket = this.socket; this.socket = undefined; this.identity = ''; socket?.close(); this.onDisconnected(); for (const p of this.pending.values()) {clearTimeout(p.timer); p.reject(new Error('Paired connection closed; admission may be unknown'));} this.pending.clear(); }

  async connect(): Promise<void> {
    this.stopped = false;
    const target = this.setting();
    if (!target?.token) throw new Error('No paired worker target is configured');
    const identity = createHash('sha256').update(target.busUrl).update('\0').update(target.token).digest('hex');
    if (this.socket && this.identity !== identity) throw new Error('Pairing changed; reconnect before dispatching');
    this.identity = identity;
    if (this.socket?.readyState === WebSocket.OPEN && !this.connecting) return;
    if (this.connecting) return this.connecting;
    this.connecting = new Promise<void>((resolve, reject) => {
      const url = new URL(target.busUrl);
      url.searchParams.set('peer', '1');
      const socket = new WebSocket(url, { headers: { Authorization: `Bearer ${target.token}` }, maxPayload: 4 * 1024 * 1024 });
      this.socket = socket;
      let settled = false;
      const timeout = setTimeout(() => socket.terminate(), 8000);
      socket.on('message', (data) => {
        let frame: { op: string; id?: string; result?: unknown; error?: string; event?: { type: string; hub?: string; data?: unknown } };
        try { frame = JSON.parse(data.toString()); } catch { return; }
        if (frame.op === 'hello') {
          socket.send(JSON.stringify({ op: 'subscribe', topics: ['agent.dispatch.update', 'agent.snapshot', 'agent.state_changed'] }));
        } else if (frame.op === 'subscribed' && !settled) {
          settled = true;
          clearTimeout(timeout);
          resolve();
          queueMicrotask(() => this.onConnected());
        } else if (frame.op === 'event' && frame.event && !frame.event.hub) {
          // The connection supplies provenance; never accept a peer's upstream event.
          this.onEvent(frame.event);
        } else if (frame.id && (frame.op === 'result' || frame.op === 'error')) {
          const pending = this.pending.get(frame.id);
          if (!pending) return;
          this.pending.delete(frame.id);
          clearTimeout(pending.timer);
          if (frame.op === 'error') pending.reject(new Error(frame.error ?? 'Paired call refused'));
          else pending.resolve(frame.result);
        }
      });
      socket.on('error', () => { /* close supplies a credential-free error */ });
      socket.on('close', () => {
        clearTimeout(timeout);
        if (this.socket !== socket) return;
        this.socket = undefined;
        this.onDisconnected();
        if (!settled) reject(new Error('Paired worker connection unavailable'));
        for (const pending of this.pending.values()) {
          clearTimeout(pending.timer);
          pending.reject(new Error('Paired connection lost; admission may be unknown'));
        }
        this.pending.clear();
        if (!this.stopped && this.setting() && !this.retry) {
          this.retry = setTimeout(() => {
            this.retry = undefined;
            void this.connect().catch(() => {});
          }, 3000);
          this.retry.unref?.();
        }
      });
    });
    try { await this.connecting; } finally { this.connecting = undefined; }
  }

  async call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    await this.connect();
    return new Promise<T>((resolve, reject) => {
      const id = `paired-${++this.sequence}`;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Paired call timed out (${method}); admission may be unknown`));
      }, 25000);
      this.pending.set(id, { resolve: (v) => resolve(v as T), reject, timer });
      this.socket!.send(JSON.stringify({ op: 'call', id, method, params }));
    });
  }
}

export const pairedWorkerConnection = new PairedWorkerConnection();

export function pairedDestinationKey(): string {
  const target = getPairedWorkerTarget();
  return target ? `paired-${createHash('sha256').update(target.busUrl).update('\0').update(target.token).digest('hex')}` : '';
}
