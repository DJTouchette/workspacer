/**
 * Main-process proxy between the renderer and the claudemon daemon.
 *
 * Owns the REST calls (spawn, input, message, approve, answer, resize, signal,
 * gate) and the SSE byte stream forwarding. Each Claude pane gets a
 * MessagePort in the renderer for byte I/O — same shape as `terminal:port`
 * for regular shells.
 */

import { BrowserWindow, MessageChannelMain, MessagePortMain } from 'electron';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { consumeSseStream } from '../lib/sseConsumer';
import { IPC } from '../shared/ipcChannels';

const BACKOFF_INITIAL_MS = 200;
const BACKOFF_MAX_MS = 5000;

interface SessionStream {
  sessionId: string;
  /** Unique key for this viewer. For spawned panes, equals sessionId.
   *  For attached viewers, equals the pane id. Multiple viewers can read
   *  from the same daemon broadcast — the daemon uses tokio::broadcast. */
  viewerKey: string;
  /** Spawned panes own the session — closing kills it. Attached viewers
   *  are just SSE consumers — closing only tears down the local stream. */
  kind: 'owner' | 'attached';
  port: MessagePortMain;
  abort: AbortController;
  stopped: boolean;
  /** IPC channel used to ship the renderer end of the port (e.g. `claude:port`
   *  for Claude panes, `terminal:port` for plain shells). */
  portChannel: string;
}

class ClaudemonSessionClient {
  private mainWindow: BrowserWindow | null = null;
  /** Keyed by viewerKey — see SessionStream.viewerKey. */
  private streams = new Map<string, SessionStream>();
  /** Initial cwd per session — used for session save/restore. claudemon's
   *  PTY is in a separate process, so we can't /proc-walk it; we just
   *  remember the spawn cwd. */
  private cwds = new Map<string, string>();

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
  }

  /** Spawn a new session (Claude pane or plain shell) via POST /sessions/spawn.
   *  `portChannel` controls which IPC channel the renderer end of the byte port
   *  is shipped on — defaults to `claude:port`; pass `terminal:port` for shells. */
  async spawn(args: {
    argv: string[];
    cwd: string;
    cols?: number;
    rows?: number;
    env?: Record<string, string>;
    portChannel?: string;
    /** Caller-pinned session id (matches `claude --session-id <uuid>`). */
    sessionId?: string;
  }): Promise<string> {
    const { portChannel = 'claude:port', sessionId: pinnedId, ...rest } = args;
    // claudemon's SpawnPayload uses snake_case.
    const reqBody = { ...rest, ...(pinnedId ? { session_id: pinnedId } : {}) };
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/spawn`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(reqBody),
    });
    if (!res.ok) {
      throw new Error(`spawn failed: HTTP ${res.status} ${await res.text()}`);
    }
    const resBody = await res.json() as { session_id: string };
    const sessionId = resBody.session_id;
    this.cwds.set(sessionId, rest.cwd);
    this.attachByteStream(sessionId, sessionId, portChannel, 'owner');
    return sessionId;
  }

  /** Attach a viewer to an already-running daemon session — no spawn, no
   *  --resume. The renderer keys ports by paneId so multiple viewers can
   *  coexist (claudemon's pty.bytes channel is a tokio::broadcast). */
  attach(paneId: string, sessionId: string, portChannel: string = 'claude:port'): string {
    if (this.streams.has(paneId)) {
      // Already attached. Defensive — useClaudeSpawn shouldn't double-attach.
      return sessionId;
    }
    this.attachByteStream(paneId, sessionId, portChannel, 'attached');
    return sessionId;
  }

  /** Detach a viewer without affecting the session itself. */
  detach(paneId: string): void {
    const stream = this.streams.get(paneId);
    if (!stream || stream.kind !== 'attached') return;
    stream.stopped = true;
    try { stream.abort.abort(); } catch {}
    try { stream.port.close(); } catch {}
    this.streams.delete(paneId);
  }

  /** Last-known cwd for a session (the spawn cwd; not live). */
  getCwd(sessionId: string): string | undefined {
    return this.cwds.get(sessionId);
  }

  /** Open a MessageChannel for byte I/O on this session and ship port2 to the renderer. */
  private attachByteStream(
    viewerKey: string,
    sessionId: string,
    portChannel: string,
    kind: 'owner' | 'attached',
  ): void {
    if (!this.mainWindow || this.mainWindow.isDestroyed()) return;

    const { port1, port2 } = new MessageChannelMain();
    const abort = new AbortController();
    const stream: SessionStream = {
      sessionId,
      viewerKey,
      kind,
      port: port1,
      abort,
      stopped: false,
      portChannel,
    };
    this.streams.set(viewerKey, stream);

    // renderer → daemon: writes from xterm come in as binary strings, we
    // forward them to POST /sessions/:id/input as raw bytes.
    port1.on('message', (event) => {
      const data = event.data;
      if (typeof data === 'string') {
        this.input(sessionId, data).catch(err =>
          console.error(`[claudemonSessionClient] input failed for ${sessionId}:`, err)
        );
      }
    });
    port1.start();

    // Renderer keys ports by viewerKey. For spawned (owner) panes, viewerKey
    // === sessionId — preserves the prior contract. For attached viewers it
    // equals the pane id, letting multiple viewers of the same session each
    // have their own port. We send all three names so the preload-side
    // `deliverPort` handler can pick whichever the channel expects.
    this.mainWindow.webContents.postMessage(
      portChannel,
      { sessionId, viewerKey, id: viewerKey },
      [port2],
    );

    // Spawn the SSE byte consumer (background).
    this.consumeByteStream(stream).catch(err =>
      console.error(`[claudemonSessionClient] byte stream ended for ${sessionId}:`, err)
    );
  }

  private async consumeByteStream(stream: SessionStream): Promise<void> {
    await consumeSseStream(
      `${CLAUDEMON_API_URL}/sessions/${stream.sessionId}/stream`,
      {
        signal: stream.abort.signal,
        backoffInitialMs: BACKOFF_INITIAL_MS,
        backoffMaxMs: BACKOFF_MAX_MS,
        joinWith: '',
        onFrame(b64) {
          const bytes = Buffer.from(b64, 'base64');
          // Send as binary string (matches the existing terminal:port shape
          // that the renderer's onTerminalOutput already decodes from).
          const binStr = bytes.toString('binary');
          if (!stream.stopped) {
            try { stream.port.postMessage(binStr); } catch {}
          }
        },
        onError: (err) => {
          console.warn(
            `[claudemonSessionClient] stream ${stream.sessionId} error, retrying:`,
            err,
          );
          // On stream error, check whether the session still exists in
          // claudemon. A 404 means the PTY process exited and the daemon
          // dropped the session — fire terminal:exit so the renderer can
          // mark the pane dead. Only the owner fires the event (attached
          // viewers don't control session lifecycle).
          if (stream.kind === 'owner' && !stream.stopped) {
            fetch(`${CLAUDEMON_API_URL}/sessions/${stream.sessionId}`)
              .then((res) => {
                if (res.status === 404 && !stream.stopped) {
                  stream.stopped = true;
                  try { stream.abort.abort(); } catch {}
                  this.streams.delete(stream.viewerKey);
                  const win = this.mainWindow;
                  if (win && !win.isDestroyed()) {
                    win.webContents.send(IPC.TERMINAL_EXIT, stream.sessionId);
                  }
                }
              })
              .catch(() => { /* best-effort: network hiccup, let the backoff retry */ });
          }
        },
      },
    );
  }

  /** Send raw bytes (or a base64-encoded payload) to the session's PTY input. */
  async input(sessionId: string, text: string): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/input`, { text, newline: false });
  }

  /** Send a chat message — only succeeds when claudemon reports mode=input. */
  async message(sessionId: string, text: string): Promise<{ ok: boolean; mode?: string }> {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}/message`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ text }),
    });
    if (res.status === 409) {
      const body = await res.json().catch(() => ({} as any)) as { mode?: string };
      return { ok: false, mode: body.mode };
    }
    if (!res.ok) throw new Error(`message HTTP ${res.status}`);
    return { ok: true };
  }

  /** Resolve a parked approval. */
  async approve(sessionId: string, decision: 'yes' | 'no' | 'always', reason?: string): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/approve`, { decision, reason });
  }

  /** Answer an AskUserQuestion picker. */
  async answer(sessionId: string, payload: { option?: number; text?: string; answers?: string[] }): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/answer`, payload);
  }

  async resize(sessionId: string, cols: number, rows: number): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/resize`, { cols, rows });
  }

  async signal(sessionId: string, signal: string): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/signal`, { signal });
  }

  async setGate(sessionId: string, on: boolean): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/gate`, { on });
  }

  async getTranscript(sessionId: string): Promise<any> {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}/transcript`);
    if (!res.ok) return { messages: [] };
    return res.json();
  }

  async getSession(sessionId: string): Promise<any> {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}`);
    if (!res.ok) return null;
    return res.json();
  }

  /** Close a session: stop all viewers and ask the daemon to terminate the PTY. */
  async close(sessionId: string): Promise<void> {
    for (const [key, s] of Array.from(this.streams.entries())) {
      if (s.sessionId !== sessionId) continue;
      s.stopped = true;
      try { s.abort.abort(); } catch {}
      try { s.port.close(); } catch {}
      this.streams.delete(key);
    }
    this.cwds.delete(sessionId);
    // Best-effort: SIGTERM the child. The session entry stays around in the
    // daemon until the next restart — fine for our purposes.
    try { await this.signal(sessionId, 'SIGTERM'); } catch {}
  }

  closeAll(): void {
    const seen = new Set<string>();
    for (const s of Array.from(this.streams.values())) {
      if (seen.has(s.sessionId)) continue;
      seen.add(s.sessionId);
      if (s.kind === 'owner') {
        this.close(s.sessionId).catch(() => {});
      } else {
        this.detach(s.viewerKey);
      }
    }
  }

  private async postJSON(path: string, body: unknown): Promise<void> {
    const res = await fetch(`${CLAUDEMON_API_URL}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      const text = await res.text().catch(() => '');
      throw new Error(`${path} HTTP ${res.status}: ${text}`);
    }
  }
}

export const claudemonSessionClient = new ClaudemonSessionClient();
