/**
 * Web build of the `window.electronAPI` surface.
 *
 * In Electron, `src/main/preload.ts` populates `window.electronAPI` via the
 * contextBridge. In a browser there is no preload, so this factory builds an
 * equivalent object backed by the hub-bus WebSocket and the bootstrap in
 * `install.ts` assigns it to `window.electronAPI`. The React app is unchanged —
 * it still calls `window.electronAPI.*` and never learns which transport it got.
 *
 * Coverage tracks the hub capability surface (`hubCapabilities.ts`). Methods the
 * hub supports (agents, terminal I/O, approvals, transcript, config, git, files,
 * provider model/detection discovery, terminal-exit + library-change events) are
 * wired; the remainder returns a safe default and warns once, to be filled in as
 * the hub RPC surface widens (Phase 3). Each remaining stub is marked `HUB-TODO`.
 * The still-stubbed surface is host-trusted/local-only work — plugin
 * install/inspect, pane-token minting, native OS dialogs/notifications, the quit
 * handshake — that a web/remote viewer can't perform against someone else's host.
 */

import type { ElectronAPI, SessionListEntry } from '../types/electron';
import type {
  ClaudeSessionSnapshot,
  AppConfig,
  AppConfigPartial,
  ClaudeProfile,
  InAppNotification,
  RecentAgentSession,
} from '../../../main/shared/ipcTypes';
import { HubBusClient, type HubEventEnvelope } from './hubBusClient';
import { mergeConversationWindow } from '../../../main/shared/mergeConversationWindow';

/** Decode a base64 PTY chunk into a binary string (1 char = 1 byte), matching
 *  the MessagePort contract the desktop's onTerminalOutput delivers. */
function decodePtyChunk(data: unknown): string {
  if (typeof data !== 'string') return '';
  try {
    return atob(data);
  } catch {
    return '';
  }
}

const warned = new Set<string>();
function warnOnce(method: string): void {
  if (warned.has(method)) return;
  warned.add(method);
  // eslint-disable-next-line no-console
  console.warn(
    `[webBackend] ${method}() is not yet available over the hub bus — returning a safe default (HUB-TODO).`,
  );
}

/** One session's shared PTY stream, however many panes are watching it. */
type PtyStream = {
  viewers: number;
  /** One debounced re-attach hook per viewer — a Set, because they coexist. */
  reprimers: Set<() => void>;
  keepalive: ReturnType<typeof setInterval>;
  attach: () => void;
};

/**
 * Refcounted PTY streams for the web build.
 *
 * The hub's attach/detach is per *session*, but a session can have several
 * viewers: a pane that spawned it, plus any watch pane opened on it from the
 * Agents or Fleet views (possibly in another workspace, where pane-level
 * dedupe can't see it). Tearing down one viewer therefore must not detach the
 * session out from under the others — that is a shared resource with a
 * refcount, not a per-viewer one.
 */
/**
 * Snapshot folding for the bus: sparse-row overlay and conversation-window
 * splicing, with the per-session cache both need.
 *
 * Two things arrive on `agent.snapshot` and neither is a complete session:
 *
 *  - **Sparse rows** from the headless brain (`sparse: true`) carry status only
 *    and must overlay the last rich row rather than clobber it.
 *  - **Rich rows** from the desktop carry a BOUNDED conversation — hubTelemetry
 *    publishes the newest turns, not the whole transcript, because this fires
 *    on every flush of every session (~60/s while one streams) and used to push
 *    megabytes a second at every bus client. Each window is anchored by
 *    `conversationOffset`, the absolute index of its first turn, so a client
 *    holding full history splices it in instead of replacing history with 12
 *    turns.
 *
 * Exported for tests, like createPtyStreams: the failure mode here is a wrong
 * transcript on the session the user is actively watching, which is invisible
 * until someone scrolls up.
 */
export function createSnapshotFold(client: Pick<HubBusClient, 'call'>) {
  const richSnaps = new Map<string, ClaudeSessionSnapshot>();
  const refetching = new Set<string>();

  /** Remember a full snapshot (from the singular `sessions.snapshot`) as the
   *  history that later windows splice onto. */
  const seedFull = (snap: ClaudeSessionSnapshot): ClaudeSessionSnapshot => {
    richSnaps.set(snap.sessionId, snap);
    return snap;
  };

  const foldSparse = (
    snap: ClaudeSessionSnapshot & { sparse?: boolean },
  ): ClaudeSessionSnapshot => {
    const prev = richSnaps.get(snap.sessionId);
    const merged = snap.sparse && prev ? { ...prev, ...snap } : snap;
    if (!snap.sparse) {
      // A rich row from the LIST call is a bounded window too — sessions.snapshots
      // is compacted now — and OverviewPane refetches that list up to 1/s while an
      // agent streams. Writing it straight into the cache would replace the
      // history the watched pane is rendering with twelve turns, once a second.
      // Merge it like any other window and keep whichever reaches further back.
      const outcome = mergeConversationWindow(prev ?? null, snap);
      if (outcome.kind === 'merged' || outcome.kind === 'adopt') {
        richSnaps.set(snap.sessionId, {
          ...snap,
          conversation: outcome.conversation,
          conversationOffset: outcome.conversationOffset,
        } as ClaudeSessionSnapshot);
      }
      // 'stale' / 'gap': what we already hold reaches further back — keep it.
    }
    if (merged.status === 'ended') richSnaps.delete(snap.sessionId);
    return merged;
  };

  /** Returns null when the push must NOT reach the UI (stale, or a gap whose
   *  refetch is now in flight). */
  const foldConversation = (snap: ClaudeSessionSnapshot): ClaudeSessionSnapshot | null => {
    const prev = richSnaps.get(snap.sessionId);
    const outcome = mergeConversationWindow(prev ?? null, snap);
    if (outcome.kind === 'stale') return null; // keep what the user can already see
    if (outcome.kind === 'gap') {
      // Turns never reached us. Refetch the full snapshot rather than render a
      // transcript that never happened; one refetch in flight per session.
      if (!refetching.has(snap.sessionId)) {
        refetching.add(snap.sessionId);
        client
          .call<ClaudeSessionSnapshot | null>('sessions.snapshot', { sessionId: snap.sessionId })
          .then((full) => {
            if (full) richSnaps.set(full.sessionId, full);
          })
          .catch(() => {})
          .finally(() => refetching.delete(snap.sessionId));
      }
      return null;
    }
    const next = {
      ...snap,
      conversation: outcome.conversation,
      conversationOffset: outcome.conversationOffset,
    } as ClaudeSessionSnapshot;
    if (next.status === 'ended') richSnaps.delete(next.sessionId);
    else richSnaps.set(next.sessionId, next);
    return next;
  };

  return { foldSparse, foldConversation, seedFull };
}

export function createPtyStreams(client: HubBusClient) {
  const streams = new Map<string, PtyStream>();

  const ensure = (sessionId: string): PtyStream => {
    const existing = streams.get(sessionId);
    if (existing) return existing;
    // attachTerminal makes claudemon replay its ring buffer (the current screen).
    const attach = () => client.call('sessions.attachTerminal', { sessionId }).catch(() => {});
    const entry: PtyStream = {
      viewers: 0,
      reprimers: new Set(),
      attach,
      // The hub lease expires after ~20s; refresh well inside that window.
      keepalive: setInterval(() => {
        client
          .call<{ ok?: boolean }>('sessions.terminalKeepalive', { sessionId })
          // ok:false means the hub already swept our lease — the forwarder is
          // gone and no bytes are coming, even though the socket is healthy and
          // nothing will fire onReconnect. Re-attach; the ring-buffer replay
          // repaints whatever was missed. Dropping this reply left a stream
          // permanently dead after a backgrounded tab throttled us past the TTL.
          .then((r) => {
            if (r && r.ok === false) attach();
          })
          .catch(() => {});
      }, 10000),
    };
    streams.set(sessionId, entry);
    return entry;
  };

  /**
   * Bracket a live PTY stream for one viewer: attach → subscribe → keepalive,
   * and return a teardown that unsubscribes and, for the last viewer only,
   * detaches. Used by both the Claude and shell terminal output subscriptions,
   * which are the byte sinks that bound a stream's lifetime in the web build.
   */
  const stream = (sessionId: string, callback: (data: string) => void): (() => void) => {
    const entry = ensure(sessionId);
    entry.viewers += 1;
    const unsub = client.subscribe(`pty.bytes.${sessionId}`, (ev: HubEventEnvelope) => {
      callback(decodePtyChunk(ev.data));
    });
    // The hub drops events past this client's buffer capacity. For PTY bytes
    // that means the terminal is now rendering a corrupted stream, so the hub
    // tells us; re-attaching replays the current screen, which repairs it.
    const unsubDesync = client.subscribe('pty.desync', (ev: HubEventEnvelope) => {
      const sid = (ev.data as { sessionId?: string } | undefined)?.sessionId;
      if (sid === sessionId) entry.attach();
    });

    // Re-prime hook: the app keeps every agent pane mounted and only toggles
    // visibility, so a terminal can initialize while its pane is hidden / zero
    // size — which eats the initial replay (it's written into a 0-col grid). When
    // the pane is shown it resizes; we treat that as the cue to re-attach and
    // replay the current screen onto the now-correctly-sized terminal. Debounced
    // so a burst of fit/resize events triggers a single replay once it settles.
    let reprimeTimer: ReturnType<typeof setTimeout> | null = null;
    const reprimer = (): void => {
      if (reprimeTimer) clearTimeout(reprimeTimer);
      reprimeTimer = setTimeout(entry.attach, 120);
    };
    entry.reprimers.add(reprimer);
    entry.attach();

    let torn = false;
    return () => {
      if (torn) return; // a double teardown must not drop the refcount twice
      torn = true;
      if (reprimeTimer) clearTimeout(reprimeTimer);
      entry.reprimers.delete(reprimer);
      unsub();
      unsubDesync();
      entry.viewers -= 1;
      if (entry.viewers > 0) return;
      clearInterval(entry.keepalive);
      streams.delete(sessionId);
      client.call('sessions.detachTerminal', { sessionId }).catch(() => {});
    };
  };

  const reprime = (sessionId: string): void => {
    const entry = streams.get(sessionId);
    if (entry) for (const r of entry.reprimers) r();
  };
  const reprimeAll = (): void => {
    for (const entry of streams.values()) for (const r of entry.reprimers) r();
  };

  return { stream, reprime, reprimeAll };
}

export function createWebBackend(token: string, busUrl?: string): ElectronAPI {
  const client = new HubBusClient(token, busUrl);
  client.start();

  // Base for the hub's HTTP routes (e.g. /plugins/settings). The web build is
  // served by the hub, so an empty base resolves relative to the page origin;
  // when an explicit bus URL is given (ws[s]://host/bus) we derive the matching
  // http[s]://host origin from it. The token authorizes the guarded routes.
  const hubHttpBase = busUrl ? busUrl.replace(/^ws/, 'http').replace(/\/bus\/?(\?.*)?$/, '') : '';
  const hubAuth = { Authorization: `Bearer ${token}` };

  // Claude panes key their byte stream + input by a "viewerKey": the sessionId
  // for a pane that spawned the session, but the *paneId* for a pane attached to
  // an already-running session (so multiple viewers can coexist). The desktop
  // preload resolves that key to a MessagePort; on the bus everything is keyed by
  // sessionId, so we map viewerKey → sessionId here (populated by attachClaude).
  const viewerSessions = new Map<string, string>();
  const sessionFor = (viewerKey: string): string => viewerSessions.get(viewerKey) ?? viewerKey;

  // Refcounted live PTY streams (see createPtyStreams). `reprime` fires the
  // debounced "re-attach + replay" hooks for a session so a freshly-shown pane
  // repaints; `reprimeAll` does it for every live stream.
  const { stream: streamPty, reprime, reprimeAll } = createPtyStreams(client);

  // Federation: which peer hub each remote session lives on, learned from
  // stamped agent.snapshot envelopes and peer-fleet seeds; plus the last
  // snapshot per remote session so a peer disconnect can push tombstones.
  const sessionHub = new Map<string, string>();
  const remoteSnaps = new Map<string, ClaudeSessionSnapshot>();
  const qualify = (sessionId: string, method: string): string => {
    const hub = sessionHub.get(sessionId);
    return hub ? `hub:${hub}/${method}` : method;
  };
  /** Merge the peers' fleets onto a local snapshot list (hub-stamped, sparse
   *  layout-ghost rows skipped — same rule as the desktop's federation seed). */
  const withPeerFleets = async (
    local: ClaudeSessionSnapshot[],
  ): Promise<ClaudeSessionSnapshot[]> => {
    let peers: Array<{ name: string; connected: boolean }> = [];
    try {
      peers = ((await client.call('federation.peers', {})) ?? []) as typeof peers;
    } catch {
      return local; // federation off (or an older hub): local fleet only
    }
    const out = [...local];
    await Promise.all(
      peers
        .filter((p) => p.connected)
        .map(async (p) => {
          try {
            const rows = ((await client.call(`hub:${p.name}/sessions.snapshots`, {})) ??
              []) as Array<ClaudeSessionSnapshot & { sparse?: boolean }>;
            for (const row of rows) {
              if (!row?.sessionId || row.sparse) continue;
              const stamped = { ...row, hub: p.name };
              sessionHub.set(row.sessionId, p.name);
              remoteSnaps.set(row.sessionId, stamped);
              out.push(stamped);
            }
          } catch {
            /* an unreachable peer costs its fleet, not the whole list */
          }
        }),
    );
    return out;
  };


  // After a reconnect the bus re-asserts topic subscriptions, but the per-stream
  // attachTerminal call (which makes claudemon replay the current screen) is not
  // re-issued — so every mirrored terminal would sit frozen until a manual
  // refresh. Re-prime each live PTY stream to re-attach and repaint.
  client.onReconnect(() => {
    reprimeAll();
  });

  // Click-through target for browser-API notification escalations (see
  // notifyEscalate below) — the notification center registers here.
  let notificationActivateCb: ((n: InAppNotification) => void) | null = null;

  // Fan hub events out to the renderer's onHubEvent subscribers. Full session
  // snapshots arrive as `agent.snapshot` events and are routed directly in
  // onClaudeSessionUpdate below.
  const hubEventHandlers = new Set<(ev: HubEventEnvelope) => void>();
  client.subscribe('*', (ev) => {
    for (const h of hubEventHandlers) h(ev);
  });

  // ── Sparse-snapshot merging ────────────────────────────────────────────
  // Two providers can produce session snapshots on the bus: the desktop app
  // (rich ClaudeSessionSnapshot rows, with conversation/tool detail) and the
  // headless brain (claudemon-backed state rows marked `sparse: true`, see
  // services/hub cmd/brain compatSnapshot). The renderer's consumers replace
  // their snapshot wholesale, so a sparse row must not clobber a rich one
  // mid-render — remember the last rich snapshot per session and overlay
  // sparse updates onto it. Ended sessions are dropped from the cache so it
  // stays bounded by the live fleet.
  const { foldSparse, foldConversation, seedFull } = createSnapshotFold(client);

  const api: ElectronAPI = {
    platform: 'web' as unknown as NodeJS.Platform,

    // No native window chrome in the browser mirror.
    setTitleBarOverlay: () => {},

    // Worktree creation shells out on the HOST; the web mirror can't. The
    // spawn dialog hides the toggle when these report not-a-repo/unavailable.
    worktreeInfo: () => Promise.resolve({ isRepo: false }),
    worktreeCreate: () =>
      Promise.resolve({ ok: false, error: 'not available over the hub bridge' }),

    // In-app updates are a desktop-shell concern; the web mirror has no feed.
    updatesGetStatus: () => Promise.resolve({ state: 'unsupported' as const, current: '' }),
    updatesCheck: () => Promise.resolve({ state: 'unsupported' as const, current: '' }),
    updatesInstall: () => Promise.resolve(),
    onUpdateStatus: () => () => {},

    // ── Shell terminals ──────────────────────────────────────────────────
    createTerminal: (shell, cwd, cols, rows) =>
      client
        .call<{ sessionId: string }>('terminals.create', { shell, cwd, cols, rows })
        .then((r) => r.sessionId),
    writeTerminal: (id, data) => {
      client.call('sessions.terminalInput', { sessionId: id, data }).catch(() => {});
    },
    resizeTerminal: (id, cols, rows) => {
      reprime(id);
      return client.call<void>('sessions.terminalResize', { sessionId: id, cols, rows });
    },
    closeTerminal: (id) =>
      client.call<void>('sessions.detachTerminal', { sessionId: id }).then(() => {}),
    onTerminalOutput: (id, callback) => streamPty(id, callback),
    // Terminal exit arrives as a flat `pty.exit` bus event carrying { sessionId };
    // the desktop fires one global callback that each pane filters by its own id,
    // so we mirror that — one subscription per listener, dispatched by sessionId.
    onTerminalExit: (callback) =>
      client.subscribe('pty.exit', (ev) => {
        const id = (ev.data as { sessionId?: string } | undefined)?.sessionId;
        if (id) callback(id);
      }),

    // ── Claude sessions ──────────────────────────────────────────────────
    spawnClaude: (opts) =>
      client.call<{ sessionId: string }>('agents.spawn', opts).then((r) => r.sessionId),
    // The bus has no config-changed event yet, so the web mirror keeps the
    // old behaviour: its snapshot refreshes on its own saves and on reload.
    onConfigChanged: () => () => {},
    claudeListModels: () => client.call('claude.listModels', {}),
    // Auto-titling runs a local headless `claude --print` in the desktop main
    // process; over the bus there is no such capability. Null = leave the name
    // alone — the desktop client titles the agent and the layout syncs it here.
    agentSuggestTitle: async () => null,
    // Reads a local transcript file; not available over the hub bus (web mirror).
    workflowAgentTranscript: async () => null,
    workflowAgentConversation: async () => null,
    // Live per-provider discovery over the bus (providers.* capabilities): the
    // managed provider's model catalog and PATH-detection status, so the web
    // Spawn dialog matches the desktop instead of falling back to free-text.
    providerListModels: (provider, cwd) => client.call('providers.listModels', { provider, cwd }),
    providerCheckAll: () => client.call('providers.checkAll', {}),
    // Keep-warm heartbeats live in the desktop's claudemon; not exposed over
    // the hub bus (settings-only surface), so the web client shows none.
    keepWarmHeartbeats: async () => [],
    claudeMessage: (sessionId, text) =>
      client.call<{ ok: boolean; mode?: string }>(qualify(sessionId, 'agents.sendMessage'), { sessionId, text }),
    claudeSetPermissionMode: (sessionId, mode) =>
      client.call<{ ok: boolean; mode?: string; error?: string }>('claude.setPermissionMode', {
        sessionId,
        mode,
      }),
    claudeSetEffort: (sessionId, effort) =>
      client.call<{ ok: boolean; effort?: string; error?: string }>('claude.setEffort', {
        sessionId,
        effort,
      }),
    claudeSetModel: (sessionId, model, effort) =>
      client.call<{ ok: boolean; error?: string }>('claude.setModel', { sessionId, model, effort }),
    claudeHandoffBrief: (sessionId) =>
      client.call<{ ok: boolean; markdown?: string; path?: string; error?: string }>(
        'claude.handoffBrief',
        { sessionId },
      ),
    claudeHandoffAgentBrief: (sessionId) =>
      client.call<{ ok: boolean; path?: string; fallback?: boolean; error?: string }>(
        'claude.handoffAgentBrief',
        { sessionId },
      ),
    claudeApprove: (sessionId, decision, reason) =>
      client.call<void>(qualify(sessionId, 'claude.approve'), { sessionId, decision, reason }).then(() => {}),
    claudeAnswer: (sessionId, payload) =>
      client.call<void>(qualify(sessionId, 'claude.answer'), { sessionId, ...payload }).then(() => {}),
    claudeResize: (sessionId, cols, rows) => {
      reprime(sessionId);
      return client.call<void>('sessions.terminalResize', { sessionId, cols, rows }).then(() => {});
    },
    claudeSignal: (sessionId, signal) =>
      client.call<void>(qualify(sessionId, 'claude.signal'), { sessionId, signal }).then(() => {}),
    claudeClose: (sessionId) =>
      client.call<void>(qualify(sessionId, 'claude.signal'), { sessionId, signal: 'SIGTERM' }).then(() => {}),
    attachClaude: (paneId, sessionId) => {
      viewerSessions.set(paneId, sessionId);
      return Promise.resolve(sessionId);
    },
    detachClaude: (paneId) => {
      viewerSessions.delete(paneId);
      return Promise.resolve();
    }, // stream lifetime owned by onClaudeOutput's teardown
    claudeGate: (sessionId, on) =>
      client.call<void>('claude.gate', { sessionId, on }).then(() => {}),
    claudeWrite: (viewerKey, data) => {
      client
        .call('sessions.terminalInput', { sessionId: sessionFor(viewerKey), data })
        .catch(() => {});
    },
    onClaudeOutput: (viewerKey, callback) => streamPty(sessionFor(viewerKey), callback),

    // ── Files (editor pane) ──────────────────────────────────────────────
    readFile: (filePath) =>
      client.call<{ path: string; contents: string; size: number }>('fs.read', { path: filePath }),
    readImagePreview: (filePath) =>
      client.call<{ path: string; dataUrl: string; width: number; height: number; size: number }>(
        'fs.readImage',
        { path: filePath },
      ),
    writeFile: (filePath, contents) =>
      client.call<{ ok: boolean }>('fs.write', { path: filePath, contents }),
    readDir: (dirPath) =>
      client.call<{ path: string; entries: { name: string; path: string; isDir: boolean }[] }>(
        'fs.listEntries',
        { path: dirPath },
      ),
    // Best effort on web: the file lives on the host, so a file:// URL only
    // works when the browser runs on the same machine. Reveal-in-folder can't
    // work at all remotely.
    fileOpenExternal: (filePath) => {
      window.open(`file://${filePath}`, '_blank');
      return Promise.resolve({ ok: true });
    },
    fileShowInFolder: () => {
      warnOnce('fileShowInFolder');
      return Promise.resolve({ ok: false, error: 'not available on web' });
    },

    // Start the host-side watch, then subscribe to the bus topic that watch
    // publishes (fs.changed, payload { path, eventType }) and filter by path.
    // Unsub stops the watch and drops the bus subscription.
    watchFile: (path, onChange) => {
      client.call('fs.watch', { path }).catch(() => {});
      const off = client.subscribe('fs.changed', (ev) => {
        const info = (ev.data ?? {}) as { path?: string; eventType?: 'change' | 'rename' };
        if (info.path === path && info.eventType)
          onChange({ path: info.path, eventType: info.eventType });
      });
      return () => {
        off();
        client.call('fs.unwatch', { path }).catch(() => {});
      };
    },

    searchProject: (opts) =>
      client.call<{
        results: { file: string; matches: { line: number; column: number; text: string }[] }[];
        truncated: boolean;
      }>('search.project', opts),

    // ── Git (review pane) ────────────────────────────────────────────────
    // The hub capabilities wrap their payloads ({ diff }, { files }, { output });
    // unwrap here so both transports present the same flat shapes to GitClient.
    gitStatus: (cwd) =>
      client.call<import('../../../main/shared/ipcTypes').GitStatus>('git.status', { cwd }),
    gitLog: (cwd, limit) =>
      client
        .call<{
          commits: import('../../../main/shared/ipcTypes').GitLogEntry[];
        }>('git.log', { cwd, limit })
        .then((r) => r.commits),
    gitDiff: (cwd, path, staged, untracked) =>
      client
        .call<{ diff: string }>('git.diff', { cwd, path, staged, untracked })
        .then((r) => r.diff),
    gitNumstat: (cwd, staged) =>
      client
        .call<{ files: import('../../../main/shared/ipcTypes').GitNumstatEntry[] }>('git.numstat', {
          cwd,
          staged,
        })
        .then((r) => r.files),
    gitCommitDiff: (cwd, hash, path) =>
      client.call<{ diff: string }>('git.commitDiff', { cwd, hash, path }).then((r) => r.diff),
    gitCommitNumstat: (cwd, hash) =>
      client
        .call<{
          files: import('../../../main/shared/ipcTypes').GitNumstatEntry[];
        }>('git.commitNumstat', { cwd, hash })
        .then((r) => r.files),
    gitStage: (cwd, path) =>
      client.call<{ output: string }>('git.stage', { cwd, path }).then((r) => r.output),
    gitUnstage: (cwd, path) =>
      client.call<{ output: string }>('git.unstage', { cwd, path }).then((r) => r.output),
    gitCommit: (cwd, message) =>
      client.call<{ output: string }>('git.commit', { cwd, message }).then((r) => r.output),
    gitPush: (cwd) => client.call<{ output: string }>('git.push', { cwd }).then((r) => r.output),

    // ── Config ───────────────────────────────────────────────────────────
    getConfig: () => client.call<AppConfig>('config.get', {}),
    reloadConfig: () => client.call<AppConfig>('config.reload', {}),
    getConfigPath: () => client.call<string>('config.getPath', {}),
    saveConfig: (partial: AppConfigPartial) => client.call<AppConfig>('config.save', partial),

    // ── Sessions / analytics / layouts ───────────────────────────────────
    listSessions: () => client.call<SessionListEntry[]>('sessions.list', {}),
    loadSession: (filename) => client.call('sessions.load', { filename }),
    saveSession: (data) => client.call<string>('sessions.save', data),
    deleteSession: (filename) => client.call<void>('sessions.delete', { filename }).then(() => {}),
    analyticsSummary: (provider, since) => client.call('analytics.summary', { provider, since }),
    analyticsRecent: (limit, provider, since) =>
      client.call('analytics.recent', { limit, provider, since }),
    layoutsList: () => client.call('layouts.list', {}),
    layoutsSave: (layout) => client.call('layouts.save', layout),
    layoutsDelete: (id) => client.call<void>('layouts.delete', { id }).then(() => {}),

    // ── Claude discovery / profiles ──────────────────────────────────────
    claudeListSessionsForDir: (cwd) => client.call('claude.sessionsForDir', { cwd }),
    claudeProfilesList: () => client.call<ClaudeProfile[]>('claude.profiles.list', {}),
    claudeProfilesAdd: (name, configDir, extraArgs, mcpItemIds) =>
      client.call<ClaudeProfile>('claude.profiles.add', { name, configDir, extraArgs, mcpItemIds }),
    claudeProfilesUpdate: (id, updates) =>
      client.call<ClaudeProfile>('claude.profiles.update', { id, updates }),
    claudeProfilesRemove: (id) =>
      client.call<void>('claude.profiles.remove', { id }).then(() => {}),
    // The SINGULAR call is the full snapshot — it is what seeds the history
    // that later bounded windows splice onto, so it must not go through the
    // sparse overlay.
    getClaudeSession: (sessionId) =>
      client
        .call<ClaudeSessionSnapshot | null>(qualify(sessionId, 'sessions.snapshot'), { sessionId })
        .then((s) => {
          if (!s) return s;
          const hub = sessionHub.get(sessionId);
          return seedFull(hub ? { ...s, hub } : s);
        }),
    getAllClaudeSessions: () =>
      client
        .call<ClaudeSessionSnapshot[]>('sessions.snapshots', {})
        .then((list) => (list || []).map(foldSparse))
        // Federation: the LOCAL call answers with the local fleet only; the
        // peers' fleets are fetched over their links and arrive hub-stamped.
        .then(withPeerFleets),
    // The daemon's full resumable-session list, enriched host-side (history DB
    // names/cost + provider auto-titles). Errors resolve to [] to match the
    // desktop handler: the Sessions pane shows nothing rather than breaking.
    listRecentAgentSessions: () =>
      client.call<RecentAgentSession[]>('sessions.recent', {}).catch(() => []),
    // Null = "can't tell" — the web client never reconciles/auto-respawns
    // agents against the daemon; the desktop owns that.
    listLiveClaudeSessionIds: () => Promise.resolve(null),
    // Federation: served by the hub-local `federation.peers` method (a browser
    // can't read peers.json). [] on an older hub or federation off.
    federationPeers: () =>
      client
        .call<Array<{ name: string; connected: boolean; lastSeen?: number }>>(
          'federation.peers',
          {},
        )
        .then((peers) => peers ?? [])
        .catch(() => []),
    onClaudeSessionUpdate: (callback) => {
      const offSnap = client.subscribe('agent.snapshot', (ev) => {
        const raw = ev.data as (ClaudeSessionSnapshot & { sparse?: boolean }) | undefined;
        if (!raw?.sessionId) return;
        // Federation: the peer name rides the ENVELOPE — stamp it onto the
        // payload (and remember it) or a remote session renders as an
        // unlabeled local-looking card with no gating.
        const snap = ev.hub ? { ...raw, hub: ev.hub, hubOffline: undefined } : raw;
        if (ev.hub) {
          sessionHub.set(snap.sessionId, ev.hub);
          remoteSnaps.set(snap.sessionId, snap);
        }
        // Sparse rows carry no conversation of their own — they overlay the
        // last rich one — so they go through foldSparse only. A rich push is a
        // bounded window and gets spliced onto the retained history.
        if (snap.sparse) {
          const merged = foldSparse(snap);
          callback(merged.sessionId, merged);
          return;
        }
        const merged = foldConversation(snap);
        if (merged) callback(merged.sessionId, merged);
      });
      // Peer link down → tombstone that hub's sessions (hubOffline, cards keep
      // rendering); link back up → the next stamped pushes clear the flag.
      const offPeer = client.subscribe('hub.peer.disconnected', (ev) => {
        const peer = (ev.data as { peer?: string } | undefined)?.peer;
        if (!peer) return;
        for (const [sessionId, hub] of sessionHub) {
          if (hub !== peer) continue;
          const last = remoteSnaps.get(sessionId);
          if (last) callback(sessionId, { ...last, hubOffline: true });
        }
      });
      return () => {
        offSnap();
        offPeer();
      };
    },

    // ── Hub plumbing ─────────────────────────────────────────────────────
    onHubEvent: (callback) => {
      hubEventHandlers.add(callback);
      return () => hubEventHandlers.delete(callback);
    },
    onHubStatus: (callback) => client.onStatus((connected) => callback({ connected })),

    // ── Shared layout document (hub-owned; tmux-style mirror) ────────────────
    // The hub provides layout.get/layout.set in-process and broadcasts
    // layout.changed; the desktop reaches these through main, the web reaches
    // them straight off the bus. Identical surface either way.
    layoutGet: () => client.call('layout.get', {}),
    layoutSet: (data) => client.call('layout.set', { data }),
    onLayoutChanged: (callback) =>
      client.subscribe('layout.changed', (ev) =>
        callback(ev.data as { version: number; data: unknown }),
      ),
    getHubStatus: () => Promise.resolve({ connected: client.isConnected() }),
    getRemoteInfo: () =>
      Promise.resolve({
        enabled: true,
        token,
        remoteUrl: location.href,
        appUrl: location.href,
        busUrl: '',
        desktopBus: false,
      }),
    // A web/remote client exists only because the host already enabled sharing,
    // and it can't restart the host's hub — so this is a no-op that reports on.
    setRemoteShare: () => {
      warnOnce('setRemoteShare');
      return Promise.resolve({
        enabled: true,
        token,
        remoteUrl: location.href,
        appUrl: location.href,
        busUrl: '',
        desktopBus: false,
      });
    },
    // No host PATH to scan from a browser — report nothing so tool gates
    // never show a false "missing" notice on the web mirror.
    toolsStatus: () => Promise.resolve([]),
    // Custom UI fonts live on the host filesystem; the web mirror can neither
    // open the native dialog nor serve workspacer-font:// — bundled fonts only.
    installUiFont: () => Promise.resolve(null),
    listUiFonts: () => Promise.resolve([]),
    // Host-only: the download writes into the desktop's config dir, and the
    // workspacer-icon:// protocol that serves it only exists there. The web
    // client still renders a derived mark, so nothing is missing visually.
    downloadProjectIcon: () =>
      Promise.resolve({
        ok: false as const,
        error: 'Icons can only be downloaded in the desktop app',
      }),
    listHubPlugins: () => {
      warnOnce('listHubPlugins');
      return Promise.resolve([]);
    },
    hubPublish: (event) =>
      client
        .call<void>('__publish', event)
        .then(() => {})
        .catch(() => {}),
    installPlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    inspectPlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    checkPluginUpdates: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    listExamplePlugins: () => {
      warnOnce('listExamplePlugins');
      return Promise.resolve([]);
    },
    installExamplePlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    removePlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    setPluginEnabled: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    // Pane tokens are minted over the hub's guarded route, exactly as the
    // desktop does it — this client holds a bearer token and already uses it for
    // the sibling /plugins/* routes. It stopped being optional when the shared
    // layout document stopped carrying busToken: a plugin pane restored from the
    // layout here has no credential baked into its URL, so this mint is its only
    // one. A weaker (view/triage) credential gets a non-ok answer and falls back
    // to null, which is exactly the behaviour this had for every caller before.
    pluginPaneToken: async (pluginId: string, agentCwd?: string) => {
      try {
        const res = await fetch(`${hubHttpBase}/plugins/pane-token`, {
          method: 'POST',
          headers: { ...hubAuth, 'Content-Type': 'application/json' },
          body: JSON.stringify({ pluginId, agentCwd: agentCwd ?? '' }),
          signal: AbortSignal.timeout(3000),
        });
        if (!res.ok) return null;
        const body = (await res.json()) as { token?: string };
        return body?.token ?? null;
      } catch {
        return null;
      }
    },
    revokePluginPaneToken: async (token: string) => {
      try {
        await fetch(`${hubHttpBase}/plugins/pane-token/revoke`, {
          method: 'POST',
          headers: { ...hubAuth, 'Content-Type': 'application/json' },
          body: JSON.stringify({ token }),
          signal: AbortSignal.timeout(5000),
        });
      } catch {
        /* best-effort; the hub also sweeps pane tokens on plugin unload */
      }
    },
    // Plugin settings live on the hub (the single source of truth, merged over
    // manifest defaults). The web client reads/writes them over the hub's guarded
    // HTTP route and hears about any edit — its own, the desktop's, or another
    // remote client's — on the plugin.settings.changed bus event.
    getPluginSettings: async (pluginId: string) => {
      try {
        const res = await fetch(
          `${hubHttpBase}/plugins/settings?pluginId=${encodeURIComponent(pluginId)}`,
          { headers: hubAuth },
        );
        if (!res.ok) return {};
        const body = (await res.json()) as { values?: Record<string, unknown> };
        return body?.values ?? {};
      } catch {
        return {};
      }
    },
    setPluginSettings: async (pluginId: string, values: Record<string, unknown>) => {
      try {
        const res = await fetch(`${hubHttpBase}/plugins/settings`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', ...hubAuth },
          body: JSON.stringify({ pluginId, values }),
        });
        // Returning the CALLER'S OWN values on a failure produced a response
        // byte-identical to a success, so even a caller that checked could not
        // tell. `null` is the failure signal.
        if (!res.ok) return null;
        const body = (await res.json()) as { values?: Record<string, unknown> };
        return body?.values ?? values;
      } catch {
        return null;
      }
    },
    onPluginSettingsChanged: (callback) =>
      client.subscribe('plugin.settings.changed', (ev) => {
        const d = ev.data as { id?: string; values?: Record<string, unknown> } | undefined;
        if (d?.id) callback(d.id, d.values ?? {});
      }),

    // ── Library ──────────────────────────────────────────────────────────
    libraryList: (cwd) => client.call('library.list', { cwd }),
    librarySave: (input) => client.call('library.save', input),
    libraryRemove: (scope, id, cwd, kind, origin) =>
      client.call<void>('library.remove', { scope, id, cwd, kind, origin }).then(() => {}),
    // The desktop mirrors every library mutation (and external edit its watcher
    // catches) onto the bus as a flat `library.changed` event; subscribe so the
    // web client auto-refreshes its prompt/skill list just like the desktop.
    onLibraryChanged: (callback) => client.subscribe('library.changed', () => callback()),

    // ── App info / dialogs ───────────────────────────────────────────────
    getCwd: () => client.call<string>('app.getCwd', {}),
    getSupervisorHome: () => client.call<string>('app.supervisorHome', {}),
    // No native OS dialog over the bus (it'd open on the host, not the viewer).
    // pickFolder opens our in-app host filesystem browser (WebFolderPicker,
    // mounted in App) by dispatching an event it resolves; fsListDir backs it.
    fsListDir: (p) => client.call('fs.listDir', { path: p }),
    pickFolder: (defaultPath) =>
      new Promise<string | null>((resolve) => {
        window.dispatchEvent(
          new CustomEvent('web:pick-folder', { detail: { defaultPath, resolve } }),
        );
      }),
    pickFiles: () => {
      const p = window.prompt('File paths to attach (comma-separated, on the host):', '');
      if (!p || !p.trim()) return Promise.resolve([]);
      return Promise.resolve(
        p
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
      );
    },
    // A browser has no host path for a dropped file — the file is on the
    // client machine, the agent runs on the host. Drops degrade to nothing
    // attached rather than to a path that doesn't exist over there.
    getPathForFile: () => '',
    // The host's clipboard is not the one the browser user pasted from, so
    // there is nothing to spill. (Attaching a browser-side paste would mean
    // uploading the bytes to the host — a different feature.)
    saveClipboardImage: () => Promise.resolve(null),
    importChromeCookies: () =>
      Promise.resolve({ imported: 0, skipped: 0, errors: ['not available on web'] }),

    // ── Lifecycle / ambient ──────────────────────────────────────────────
    onBeforeQuit: () => () => {},
    notifyQuitSaved: (_ok?: boolean) => {}, // no quit handshake in the browser
    setActiveSession: () => {
      /* no ambient OS notifications on web */
    },
    onFocusAgent: () => () => {},
    onSystemNotice: () => () => {}, // host-process notices; not relevant to the web client
    // Main-process notification mirror; web builds still get plugin/bus
    // notifications via onHubEvent's `notify.post` path.
    onInAppNotification: () => () => {},
    // Escalation parity via the browser Notification API: a hidden tab still
    // taps the user on the shoulder. Permission is requested lazily on the
    // first escalation attempt; clicks focus the tab and re-enter the
    // notification center's activate path via the registered callback.
    notifyEscalate: (n) => {
      if (typeof Notification === 'undefined') return;
      const show = () => {
        const note = new Notification(n.title, {
          body: n.body,
          tag: n.key ?? n.id, // same-key escalations collapse like the center
        });
        note.onclick = () => {
          window.focus();
          notificationActivateCb?.(n);
          note.close();
        };
      };
      if (Notification.permission === 'granted') show();
      else if (Notification.permission === 'default') {
        void Notification.requestPermission().then((p) => {
          if (p === 'granted') show();
        });
      }
    },
    onNotificationActivate: (callback) => {
      notificationActivateCb = callback;
      return () => {
        if (notificationActivateCb === callback) notificationActivateCb = null;
      };
    },
    openLogsFolder: () => {
      warnOnce('openLogsFolder');
      return Promise.resolve({ ok: false, error: 'not available on web' });
    },
    installCli: () => {
      warnOnce('installCli');
      return Promise.resolve({ ok: false, message: 'not available on web' });
    },
    // Model-rate overrides live in a host file; the web client can't read/write it.
    pricingGetRates: () => {
      warnOnce('pricingGetRates');
      return Promise.resolve({ defaults: {}, overrides: {} });
    },
    pricingSaveOverrides: () => {
      warnOnce('pricingSaveOverrides');
      return Promise.resolve({ ok: false });
    },
  };

  return api;
}
