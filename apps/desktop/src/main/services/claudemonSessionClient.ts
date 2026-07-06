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

/**
 * Signal names we are willing to forward to the daemon (SECURITY.md #9). Both the
 * `claude:signal` IPC handler and the `claude.signal` hub capability funnel through
 * `signal()` below, so validating here gates every desktop path (renderer and
 * remote/MCP bus alike) — a caller can't push an arbitrary string into the daemon's
 * signal endpoint. claudemon itself is stricter still: its `Signal` enum only
 * accepts SIGINT/SIGTERM/SIGKILL, so SIGSTOP/SIGCONT would be rejected there today;
 * we keep the doc's recommended superset so the allowlist stays correct if the
 * daemon later grows job-control signals.
 */
const ALLOWED_SIGNALS = new Set(['SIGTERM', 'SIGINT', 'SIGKILL', 'SIGSTOP', 'SIGCONT']);

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
  /** Managed (adapter-driven) sessions we spawned. They have no PTY byte
   *  stream, so they're invisible to the `streams` map — track them here so
   *  closeAll() can terminate them too (the daemon maps SIGTERM on a managed
   *  session to a provider terminate). */
  private managedIds = new Set<string>();

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
    /** Hybrid agents (e.g. 'codex'): the PTY is the agent's TUI and claudemon
     *  also tails its rollout transcript to drive the GUI conversation view. */
    rolloutProvider?: string;
  }): Promise<string> {
    const { portChannel = 'claude:port', sessionId: pinnedId, rolloutProvider, ...rest } = args;
    // claudemon's SpawnPayload uses snake_case.
    const reqBody = {
      ...rest,
      ...(pinnedId ? { session_id: pinnedId } : {}),
      ...(rolloutProvider ? { rollout_provider: rolloutProvider } : {}),
    };
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/spawn`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(reqBody),
    });
    if (!res.ok) {
      throw new Error(`spawn failed: HTTP ${res.status} ${await res.text()}`);
    }
    const resBody = (await res.json()) as { session_id: string };
    const sessionId = resBody.session_id;
    this.cwds.set(sessionId, rest.cwd);
    this.attachByteStream(sessionId, sessionId, portChannel, 'owner');
    return sessionId;
  }

  /** Spawn a *managed* (adapter-driven) session via POST /sessions/spawn-managed.
   *  Unlike `spawn`, there's no PTY/byte stream — claudemon drives the provider's
   *  own API (e.g. `opencode serve`) and the renderer observes via the session
   *  snapshot/conversation/status streams like a Claude GUI session. */
  async spawnManaged(args: {
    /** Provider backend. 'claude' means the headless stream-json adapter
     *  (`claude --print --input-format stream-json --output-format stream-json`)
     *  — PTY Claude never goes through spawn-managed (it stays on `spawn`). */
    provider: 'opencode' | 'codex' | 'pi' | 'claude';
    cwd: string;
    model?: string;
    /** Reasoning-effort level (codex `model_reasoning_effort`); others ignore it. */
    effort?: string;
    /** Resolved launcher binary (the desktop resolves it on PATH). */
    bin?: string;
    /** YOLO / skip approvals — auto-approve every command and file change. */
    yolo?: boolean;
    /** Claude (stream) only: the full Claude permission mode
     *  (default/acceptEdits/plan/bypassPermissions) — `--permission-mode`. */
    permissionMode?: string;
    /** Claude (stream) only: resume this prior conversation (`--resume <id>`). */
    resumeSessionId?: string;
    /** Claude (stream) only: extra argv appended verbatim (profile extras,
     *  session-scoped `--mcp-config`, …). */
    extraArgs?: string[];
    /** Claude (stream) only: env merged over the daemon's environment
     *  (e.g. a profile's CLAUDE_CONFIG_DIR). */
    env?: Record<string, string>;
    /** Workspacer MCP facade URL to register with the provider (supervisors). */
    mcp?: string;
    /** Role instructions to prepend to the agent's first turn (supervisors). */
    instructions?: string;
    sessionId?: string;
  }): Promise<string> {
    const { sessionId: pinnedId, permissionMode, resumeSessionId, extraArgs, ...rest } = args;
    // claudemon's SpawnManagedPayload uses snake_case for multi-word fields;
    // the resume id rides its `resume` field.
    const reqBody = {
      ...rest,
      ...(pinnedId ? { session_id: pinnedId } : {}),
      ...(permissionMode ? { permission_mode: permissionMode } : {}),
      ...(resumeSessionId ? { resume: resumeSessionId } : {}),
      ...(extraArgs?.length ? { extra_args: extraArgs } : {}),
    };
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/spawn-managed`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(reqBody),
    });
    if (!res.ok) {
      throw new Error(`spawn-managed failed: HTTP ${res.status} ${await res.text()}`);
    }
    const resBody = (await res.json()) as { session_id: string };
    this.cwds.set(resBody.session_id, rest.cwd);
    this.managedIds.add(resBody.session_id);
    return resBody.session_id;
  }

  /** List the models a managed provider can launch with, live-queried from its
   *  CLI/server via GET /providers/:provider/models. Returns an empty list on
   *  any failure so the spawn dialog can fall back to free-text entry. */
  async listProviderModels(
    provider: 'opencode' | 'codex' | 'pi',
    cwd?: string,
    bin?: string,
  ): Promise<Array<{ id: string; label: string; default: boolean }>> {
    const params = new URLSearchParams();
    if (cwd) params.set('cwd', cwd);
    if (bin) params.set('bin', bin);
    const qs = params.toString();
    try {
      const res = await fetch(
        `${CLAUDEMON_API_URL}/providers/${provider}/models${qs ? `?${qs}` : ''}`,
      );
      if (!res.ok) return [];
      const body = (await res.json()) as {
        models?: Array<{ id: string; label: string; default?: boolean }>;
      };
      return (body.models ?? []).map((m) => ({
        id: m.id,
        label: m.label,
        default: m.default === true,
      }));
    } catch {
      return [];
    }
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
    // A restored pane can point at a session that is stopped (the daemon keeps
    // terminated sessions as resumable rows, so the id still resolves) or gone
    // entirely. Without a liveness check the viewer sits silently on a dead
    // stream and the pane looks alive — verify and surface the exit instead.
    this.verifyAttachTarget(paneId, sessionId);
    return sessionId;
  }

  /** Fire-and-forget: if the attach target is missing or stopped, tear the
   *  viewer down and tell the renderer the session is dead (keyed by the
   *  viewer's pane id, which is how attached panes listen). */
  private verifyAttachTarget(viewerKey: string, sessionId: string): void {
    fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}`)
      .then(async (res) => {
        let dead = res.status === 404;
        if (res.ok) {
          const body = (await res.json().catch(() => null)) as { mode?: string } | null;
          dead = body?.mode === 'stopped';
        }
        if (!dead) return;
        const stream = this.streams.get(viewerKey);
        if (stream && stream.sessionId === sessionId) {
          stream.stopped = true;
          try {
            stream.abort.abort();
          } catch {}
          try {
            stream.port.close();
          } catch {}
          this.streams.delete(viewerKey);
        }
        const win = this.mainWindow;
        if (win && !win.isDestroyed()) {
          win.webContents.send(IPC.TERMINAL_EXIT, viewerKey);
        }
      })
      .catch(() => {
        /* daemon unreachable — the SSE backoff handles that path */
      });
  }

  /** Detach a viewer without affecting the session itself. */
  detach(paneId: string): void {
    const stream = this.streams.get(paneId);
    if (!stream || stream.kind !== 'attached') return;
    stream.stopped = true;
    try {
      stream.abort.abort();
    } catch {}
    try {
      stream.port.close();
    } catch {}
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
        this.input(sessionId, data).catch((err) =>
          console.error(`[claudemonSessionClient] input failed for ${sessionId}:`, err),
        );
      }
    });
    port1.start();

    // Renderer keys ports by viewerKey. For spawned (owner) panes, viewerKey
    // === sessionId — preserves the prior contract. For attached viewers it
    // equals the pane id, letting multiple viewers of the same session each
    // have their own port. We send all three names so the preload-side
    // `deliverPort` handler can pick whichever the channel expects.
    this.mainWindow.webContents.postMessage(portChannel, { sessionId, viewerKey, id: viewerKey }, [
      port2,
    ]);

    // Spawn the SSE byte consumer (background).
    this.consumeByteStream(stream).catch((err) =>
      console.error(`[claudemonSessionClient] byte stream ended for ${sessionId}:`, err),
    );
  }

  private async consumeByteStream(stream: SessionStream): Promise<void> {
    await consumeSseStream(`${CLAUDEMON_API_URL}/sessions/${stream.sessionId}/stream`, {
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
          try {
            stream.port.postMessage(binStr);
          } catch {}
        }
      },
      onError: (err) => {
        console.warn(`[claudemonSessionClient] stream ${stream.sessionId} error, retrying:`, err);
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
                try {
                  stream.abort.abort();
                } catch {}
                this.streams.delete(stream.viewerKey);
                const win = this.mainWindow;
                if (win && !win.isDestroyed()) {
                  win.webContents.send(IPC.TERMINAL_EXIT, stream.sessionId);
                }
              }
            })
            .catch(() => {
              /* best-effort: network hiccup, let the backoff retry */
            });
        }
      },
    });
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
      const body = (await res.json().catch(() => ({}) as any)) as { mode?: string };
      return { ok: false, mode: body.mode };
    }
    if (!res.ok) throw new Error(`message HTTP ${res.status}`);
    return { ok: true };
  }

  /**
   * Live-switch the session's permission mode without a restart. claudemon
   * drives + verifies the switch (claude: shift+tab cycle against the screen;
   * codex: the adapter's approval flag) — `ok: false` means it genuinely can't
   * be done live (busy, not in the cycle, bypass-spawned) and the caller
   * should offer the restart path. `mode` reports the daemon-confirmed mode.
   */
  async setPermissionMode(
    sessionId: string,
    mode: string,
  ): Promise<{ ok: boolean; mode?: string; error?: string }> {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}/permission-mode`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ mode }),
    });
    const body = (await res.json().catch(() => ({}) as any)) as { mode?: string; error?: string };
    if (res.ok) return { ok: true, mode: body.mode ?? mode };
    return { ok: false, mode: body.mode, error: body.error ?? `HTTP ${res.status}` };
  }

  /**
   * Live-switch a managed session's model (and/or reasoning effort) without a
   * restart — codex applies it to the running thread (`thread/settings/update`).
   * `ok: false` means the provider can't do it live (opencode/pi, codex rollout
   * fallback) and the caller should offer the restart path. Claude sessions
   * don't use this — they switch via the `/model` slash command on the message
   * path.
   */
  async setModel(
    sessionId: string,
    model?: string,
    effort?: string,
  ): Promise<{ ok: boolean; error?: string }> {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}/model`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ model, effort }),
    });
    const body = (await res.json().catch(() => ({}) as any)) as { error?: string };
    if (res.ok) return { ok: true };
    return { ok: false, error: body.error ?? `HTTP ${res.status}` };
  }

  /**
   * Build (and persist under ~/.workspacer/handoffs/) a cross-provider
   * handoff brief for a session — the markdown a successor agent of any
   * harness reads to take the work over.
   */
  async handoffBrief(
    sessionId: string,
  ): Promise<{ ok: boolean; markdown?: string; path?: string; error?: string }> {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}/handoff`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({}),
    });
    const body = (await res.json().catch(() => ({}) as any)) as {
      markdown?: string;
      path?: string;
      error?: string;
    };
    if (res.ok) return { ok: true, markdown: body.markdown, path: body.path };
    return { ok: false, error: body.error ?? `HTTP ${res.status}` };
  }

  /** Resolve a parked approval. */
  async approve(
    sessionId: string,
    decision: 'yes' | 'no' | 'always',
    reason?: string,
  ): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/approve`, { decision, reason });
  }

  /** Answer an AskUserQuestion picker. */
  async answer(
    sessionId: string,
    payload: { option?: number; text?: string; answers?: string[] },
  ): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/answer`, payload);
  }

  async resize(sessionId: string, cols: number, rows: number): Promise<void> {
    await this.postJSON(`/sessions/${sessionId}/resize`, { cols, rows });
  }

  async signal(sessionId: string, signal: string): Promise<void> {
    if (!ALLOWED_SIGNALS.has(signal)) {
      throw new Error(`refusing to forward unrecognized signal "${signal}"`);
    }
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

  /** Parsed conversation items + the latest sequence number. Pass `sinceSeq` to
   *  fetch only items after that sequence — cheap incremental polling. */
  async getConversation(
    sessionId: string,
    sinceSeq?: number,
  ): Promise<{ seq: number; items: any[] }> {
    const qs = typeof sinceSeq === 'number' ? `?since=${sinceSeq}` : '';
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}/conversation${qs}`);
    if (!res.ok) return { seq: 0, items: [] };
    const data = (await res.json()) as { seq?: number; items?: unknown };
    return { seq: data.seq ?? 0, items: Array.isArray(data.items) ? data.items : [] };
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
      try {
        s.abort.abort();
      } catch {}
      try {
        s.port.close();
      } catch {}
      this.streams.delete(key);
    }
    this.cwds.delete(sessionId);
    this.managedIds.delete(sessionId);
    // Best-effort: SIGTERM the child (for managed sessions the daemon maps
    // this to a provider terminate). The session entry stays around in the
    // daemon as a resumable Stopped row — that's intended.
    try {
      await this.signal(sessionId, 'SIGTERM');
    } catch {}
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
    // Managed sessions have no byte stream, so the loop above never sees them.
    for (const id of Array.from(this.managedIds)) {
      if (seen.has(id)) continue;
      this.close(id).catch(() => {});
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
