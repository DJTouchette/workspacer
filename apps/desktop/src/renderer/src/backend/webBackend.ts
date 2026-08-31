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
import type { ConversationTurn } from '../types/claudeSession';
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
import { mergeSelectionSlice, readSelectionSlice } from '../../../main/shared/canonicalSelection';
import {
  createBusConversations,
  foldConversationItemsToTurns,
  type ConversationDeltaWire,
  type ConversationSnapWire,
} from './busConversation';
import {
  openBrowserFilePicker,
  reportAttachmentFailures,
  uploadAttachments,
  UPLOAD_TIMEOUT_MS,
} from '../lib/attachmentUpload';
import { postNotification } from '../lib/notificationBus';
import { launchPermissionMode } from '../lib/providerCaps';
import { isLoopbackOrigin } from '../lib/pluginOrigin';
import type { PluginManifest } from '../types/plugin';

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

function transcriptLineText(turn: ConversationTurn): string {
  const chunks: string[] = [];
  if (turn.content.trim()) chunks.push(turn.content);
  for (const tc of turn.toolCalls ?? []) {
    chunks.push(`⚙ ${tc.name}`);
    if (typeof tc.response === 'string' && tc.response.trim()) {
      chunks.push(`↳ ${tc.response.slice(0, 400)}`);
    }
  }
  return chunks.join('\n').trim();
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
/** What `agents.spawn` answered about a session this client started: the mode
 *  it runs under, whether that is full access, and any escalation the hub
 *  refused. Recorded by `noteLaunch`, folded onto the session's snapshots. */
export type LaunchTruth = {
  permissionMode: string;
  fullAccess: boolean;
  escalationScrubbed?: string[];
};

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
  const launchTruth = new Map<string, LaunchTruth>();

  /**
   * Remember what `agents.spawn` ANSWERED about a session we just started.
   *
   * The hub stamps `fullAccess` on every spawn result — what the session
   * actually runs with, as opposed to what was asked for — and it reaches
   * exactly one place: the promise this client is holding. Nothing else on the
   * bus repeats it in time: the brain does record the same truth onto its rows
   * (cmd/brain/enrich.go noteLaunch), but the pane paints the moment the spawn
   * resolves, and the first snapshot to carry that overlay is one claudemon
   * event away. Between those two the mode pill had nothing and invented a
   * default — which is precisely the window in which someone who just clicked
   * "Full access" is looking at it.
   *
   * So the answer is folded into the snapshot the pane already reads, in the
   * fields it already reads, rather than being threaded through a second
   * channel the pills would have to learn about.
   */
  const noteLaunch = (sessionId: string, truth: LaunchTruth) => {
    if (!sessionId) return;
    launchTruth.set(sessionId, truth);
  };

  /** Fill, never overwrite: a row that already carries these knows at least as
   *  much as we do (the brain's overlay is this same value, and a rich desktop
   *  snapshot knows more). */
  const applyLaunch = <T extends ClaudeSessionSnapshot>(snap: T): T => {
    const truth = launchTruth.get(snap.sessionId);
    if (!truth) return snap;
    const settings = { ...snap.settings };
    if (settings.permissionMode === undefined) settings.permissionMode = truth.permissionMode;
    if (settings.bypassAvailable === undefined) settings.bypassAvailable = truth.fullAccess;
    return {
      ...snap,
      settings,
      ...(snap.escalationScrubbed === undefined &&
        truth.escalationScrubbed?.length && { escalationScrubbed: truth.escalationScrubbed }),
    };
  };

  /** Remember a full snapshot (from the singular `sessions.snapshot`) as the
   *  history that later windows splice onto. */
  const seedFull = (snap: ClaudeSessionSnapshot): ClaudeSessionSnapshot => {
    const mapped = applySelection(snap, richSnaps.get(snap.sessionId));
    richSnaps.set(mapped.sessionId, mapped);
    return applyLaunch(mapped);
  };

  /**
   * Carry the daemon-owned canonical selection slice across a sparse overlay.
   *
   * `{...prev, ...snap}` is only accidentally safe for it. The combined brain
   * emits the camelCase pair alongside claudemon's `requested_selection` /
   * `resolved_context_window`, but an OLDER one sends only the snake spelling
   * (as `status_line` and `tool_calls` still do), and against that row the
   * spread neither maps the fields NOR protects the pair a richer earlier row
   * supplied. Mapping and presence-merging both is what makes a sparse update
   * able to add an owner fact and unable to subtract one.
   */
  const applySelection = <T extends ClaudeSessionSnapshot>(
    snap: T,
    prev?: ClaudeSessionSnapshot,
  ) => {
    const slice = mergeSelectionSlice(prev, readSelectionSlice(snap));
    return slice.requestedSelection === undefined && slice.resolvedContextWindow === undefined
      ? snap
      : { ...snap, ...slice };
  };

  const foldSparse = (
    snap: ClaudeSessionSnapshot & { sparse?: boolean },
  ): ClaudeSessionSnapshot => {
    const prev = richSnaps.get(snap.sessionId);
    const merged = applyLaunch(
      applySelection(snap.sparse && prev ? { ...prev, ...snap } : snap, prev),
    );
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
    if (merged.status === 'ended') {
      richSnaps.delete(snap.sessionId);
      // Same retention as the snapshot cache: the launch truth describes a
      // process that has exited, so it is bounded by the LIVE fleet, not by
      // how long this tab has been open.
      launchTruth.delete(snap.sessionId);
    }
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
    const next = applySelection(
      {
        ...snap,
        conversation: outcome.conversation,
        conversationOffset: outcome.conversationOffset,
      } as ClaudeSessionSnapshot,
      prev,
    );
    if (next.status === 'ended') {
      richSnaps.delete(next.sessionId);
      launchTruth.delete(next.sessionId);
    } else richSnaps.set(next.sessionId, next);
    return applyLaunch(next);
  };

  return { foldSparse, foldConversation, seedFull, noteLaunch };
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
  // The same endpoint spelled absolutely — what a plugin's guest URL has to be
  // built against, since a relative base means nothing inside an <iframe>/<webview>
  // that isn't ours. Empty base ⇒ the hub served this very page.
  const hubOrigin = hubHttpBase || (typeof location !== 'undefined' ? location.origin : '');
  // Is the hub on THIS machine? The one observable fact that decides whether a
  // sidecar's loopback port is reachable from here — not `platform === 'web'`,
  // which is also true of a browser on the hub host and false for a desktop
  // remote client pointed at someone else's machine.
  const hubIsThisMachine = isLoopbackOrigin(hubOrigin);

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
  const readProviderSubagentConversation = async (
    sessionId: string,
    runId: string | null,
    agentId: string,
  ): Promise<ConversationTurn[] | null> => {
    if (runId !== null || !sessionId || !agentId) return null;
    try {
      const res = await client.call<ConversationSnapWire | null>(
        qualify(sessionId, 'sessions.subagentConversation'),
        { sessionId, agentId },
      );
      if (!res || !Array.isArray(res.items)) return null;
      const fallbackTs = Date.now();
      return foldConversationItemsToTurns(res.items).map((turn) => ({
        ...turn,
        timestamp: turn.timestamp ?? fallbackTs,
      }));
    } catch {
      return null;
    }
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
  const { foldSparse, foldConversation, seedFull, noteLaunch } = createSnapshotFold(client);

  // ── Conversation for conversation-less rows ────────────────────────────
  // A sparse (headless-brain) row carries no transcript at all — brain keeps
  // it out of the snapshot on purpose and expects clients to fetch
  // `sessions.conversation` themselves (the phone does; see busConversation.ts
  // for the whole story). Without this, `/app` against a `workspacer serve`
  // node showed an empty chat and an optimistic "Sending…" bubble that could
  // never retire, because the turn it waits for lives only in that endpoint.
  //
  // `sessionUpdateCbs` is every live onClaudeSessionUpdate subscriber: a fetch
  // that lands between snapshots has to reach the renderer on its own, so the
  // fold re-emits the session's newest snapshot with the transcript merged in.
  const sessionUpdateCbs = new Set<(sessionId: string, snap: ClaudeSessionSnapshot) => void>();
  const lastSnaps = new Map<string, ClaudeSessionSnapshot>();
  // Sessions some pane actually opened (getClaudeSession). Only these are
  // polled — a 40-agent fleet's cards need status, not forty transcripts.
  const watchedSessions = new Set<string>();
  const conversations = createBusConversations(
    (sessionId, params) => client.call(qualify(sessionId, 'sessions.conversation'), params),
    (sessionId) => {
      const last = lastSnaps.get(sessionId);
      if (!last) return;
      const merged = conversations.merge(last);
      for (const cb of sessionUpdateCbs) cb(sessionId, merged);
    },
  );
  // ── Streaming cadence ──────────────────────────────────────────────────
  // Fetching on every `agent.snapshot` is the right TRIGGER and, on its own,
  // far too coarse a CLOCK. A headless node publishes a session row when the
  // session's STATE changes, and for a managed (stream-transport) session
  // claudemon only broadcasts a SessionUpdate on a mode transition — reply
  // text growing is not one. Measured against a local `workspacer serve`: a
  // 22-second turn produced 33 conversation deltas inside claudemon and
  // exactly TWO bus snapshots, one at `responding` (7ms after the send) and
  // one at `input`. The client therefore rendered ONCE, at the end: median
  // 11.3s behind the daemon, worst case 21.8s — the whole turn. Short replies
  // land fast and long ones look dead, which is exactly the "some are fast,
  // some are slow" the report described.
  //
  // So while a watched session is actually streaming, tick on our own clock.
  // Frequency was the whole problem: the `?since` anchor was already carrying
  // one item per fetch, not the transcript. Same turn, on the clock below:
  // median 222ms behind the daemon, worst case 499ms, 30 renders instead of 1.
  //
  // What it costs, measured rather than assumed: 46 fetches / 62 KB where the
  // edge trigger alone did 5 / 4.7 KB, for a 2.6 KB reply. The amplification is
  // structural, not a bug in the anchor — claudemon coalesces a streaming reply
  // into ONE item that grows in place, and `/conversation` answers with items,
  // so each fetch re-sends the whole in-progress message. It is therefore
  // quadratic in reply length, and the only way off that curve is a real delta
  // feed on the bus. THAT FEED NOW EXISTS — see the delta-push block below —
  // and this clock is its FALLBACK, not its supplement: it keeps running for a
  // session exactly until the push path proves itself (an old hub without the
  // demand table, an old node without the forwarder, a federated session whose
  // deltas would have to cross the peer link), and stops the moment it does.
  //
  // Idle sessions get NO timer: the mode transition that starts a turn is
  // published within single-digit milliseconds (7ms, measured, after the send
  // acks), so the edge trigger already covers "a turn began", and ticking on
  // idle panes would re-download that last large item forever for nothing.
  const STREAMING_POLL_MS = 500;
  const pollTimers = new Map<string, { timer: ReturnType<typeof setTimeout>; every: number }>();
  const stopPolling = (sessionId: string): void => {
    const cur = pollTimers.get(sessionId);
    if (!cur) return;
    clearTimeout(cur.timer);
    pollTimers.delete(sessionId);
  };
  /**
   * Bring the session's own poll clock in line with the snapshot we just saw.
   * Idempotent: a snapshot that doesn't change the pace leaves the pending
   * timer alone, so a burst of state ticks can't restart (and so defer) it.
   */
  const pacePolling = (sessionId: string): void => {
    const snap = lastSnaps.get(sessionId);
    // A session no pane opened, one that ended, or one whose row carries its
    // own transcript (a rich desktop publisher is on the bus for it) has
    // nothing for this clock to do.
    const wanted =
      watchedSessions.has(sessionId) &&
      snap &&
      !Array.isArray(snap.conversation) &&
      // Push is live for this session: deltas arrive the moment claudemon
      // parses them, and every tick would re-download the growing reply.
      !convPush.get(sessionId)?.live
        ? snap.ambientState === 'streaming' || snap.ambientState === 'thinking'
          ? STREAMING_POLL_MS
          : 0
        : 0;
    if (!wanted) return stopPolling(sessionId);
    const cur = pollTimers.get(sessionId);
    if (cur?.every === wanted) return;
    if (cur) clearTimeout(cur.timer);
    const timer = setTimeout(() => {
      pollTimers.delete(sessionId);
      if (!watchedSessions.has(sessionId)) return;
      void conversations.poke(sessionId);
      pacePolling(sessionId);
    }, wanted);
    pollTimers.set(sessionId, { timer, every: wanted });
  };

  // ── Delta push ─────────────────────────────────────────────────────────
  // The long-term fix the clock above was standing in for: while a pane has a
  // local headless session open, subscribe to `agent.conversation.<id>`. The
  // hub counts exactly these exact-topic subscriptions (internal/bus/demand.go)
  // and tells the brain, which forwards claudemon's own `/conversation/stream`
  // fragments for exactly the demanded sessions — the subscription IS the
  // demand, and the socket dying is the unsubscribe, so a closed laptop lid
  // cannot leave a transcript firehose running.
  //
  // VERSION SKEW: a new client against an old hub (no demand table) or an old
  // node (no forwarder) subscribes successfully and simply never hears
  // anything — indistinguishable from "push is live, session is idle" without
  // a positive signal. The brain therefore publishes a `ready` handshake on
  // the first demand, and until that (or any delta) arrives the poll tick
  // keeps running: degraded to exactly the pre-push behaviour, never broken.
  //
  // Measured (real serve + real /app bundle in headless Chromium, ~2.3 KB
  // reply, same client bundle both runs): push 19ms median / 34ms worst DOM
  // lag and 12.3 KB of transcript on the wire; the tick fallback against a
  // master-built hub 260ms / 510ms and 56.1 KB — and the fallback engaged by
  // itself, which is the skew path proven live. Push is O(reply); the tick's
  // re-downloads are O(reply²).
  type ConvPush = { off: () => void; live: boolean };
  const convPush = new Map<string, ConvPush>();
  /** The delta fold rule for this session, read off its snapshot: a managed
   *  (stream-transport / non-claude) session streams fragments that extend the
   *  open bubble; a Claude PTY transcript re-emits whole blocks. Mirrors
   *  conversationApplier.ts's `streaming` test. */
  const deltaStreaming = (sessionId: string): boolean => {
    const snap = lastSnaps.get(sessionId) as
      (ClaudeSessionSnapshot & { provider?: string }) | undefined;
    return snap?.transport === 'stream' || (!!snap?.provider && snap.provider !== 'claude');
  };
  const armPush = (sessionId: string): void => {
    // Federated sessions stay on the tick: their deltas would have to cross
    // the peer link, which forwards only the classified fleet topics and has
    // no demand propagation. The tick already works there.
    if (convPush.has(sessionId) || sessionHub.has(sessionId)) return;
    const entry: ConvPush = { live: false, off: () => {} };
    entry.off = client.subscribe(`agent.conversation.${sessionId}`, (ev) => {
      const d = ev.data as ConversationDeltaWire | undefined;
      if (!d) return;
      if (!entry.live) {
        // Ready (or a first delta): the push path exists end to end on THIS
        // hub and THIS node — it travelled it. Disarm the fallback clock.
        entry.live = true;
        stopPolling(sessionId);
      }
      if (typeof d.seq === 'number') {
        conversations.applyDelta(sessionId, d, deltaStreaming(sessionId));
      }
    });
    convPush.set(sessionId, entry);
  };
  const disarmPush = (sessionId: string): void => {
    const entry = convPush.get(sessionId);
    if (!entry) return;
    entry.off();
    convPush.delete(sessionId);
  };
  client.onReconnect(() => {
    // The bus client re-asserts the topic subscription itself and the hub
    // re-counts the demand — but we may have reconnected to a hub or through a
    // path where push does not exist, so the proof is void until a fresh
    // `ready` (the brain re-announces on the 0→1 our re-subscribe causes).
    // Drop to the tick meanwhile.
    for (const [sessionId, entry] of convPush) {
      entry.live = false;
      pacePolling(sessionId);
    }
  });

  /** Remember a snapshot on its way to the renderer and overlay the folded
   *  transcript when the row brought none. */
  const withConversation = (snap: ClaudeSessionSnapshot): ClaudeSessionSnapshot => {
    if (!snap?.sessionId) return snap;
    lastSnaps.set(snap.sessionId, snap);
    const merged = conversations.merge(snap);
    if (snap.status === 'ended') {
      // The final render keeps the transcript we already folded (it is in
      // `merged`), but nothing more is coming — stop polling, drop the delta
      // subscription (which releases the hub-side demand), and let the caches
      // go, so they stay bounded by the LIVE fleet like foldSparse's.
      watchedSessions.delete(snap.sessionId);
      conversations.forget(snap.sessionId);
      lastSnaps.delete(snap.sessionId);
      stopPolling(snap.sessionId);
      disarmPush(snap.sessionId);
    } else {
      pacePolling(snap.sessionId);
    }
    return merged;
  };

  /**
   * `pickFiles` without `attachment` means the caller wants a path ON THE HOST
   * — the editor's fallback open, the custom-binary browsers in the spawn
   * dialog and settings. A browser cannot produce one, and the answer this
   * replaced (a `window.prompt` captioned "on the host") was a lie dressed as a
   * feature: whatever was typed went through unchecked as if a dialog had
   * returned it. Refuse, say why, and leave the text field those callers sit
   * next to as the way to enter a path deliberately.
   */
  const refuseHostFilePick = (): Promise<string[]> => {
    postNotification({
      level: 'warn',
      title: 'Choosing a file on the server needs the desktop app',
      body: 'A browser can only see files on this machine. Type the path on the server instead.',
    });
    return Promise.resolve([]);
  };

  /**
   * The `pickFiles` implementation for clients with no host filesystem: the
   * browser's own picker, then `files.upload` per file. Refusals and failures
   * are reported rather than returned, because `pickFiles` answers a `string[]`
   * and a shorter-than-expected array is precisely the silent failure this
   * replaces.
   */
  const pickAndUpload = async (sessionId?: string): Promise<string[]> => {
    const files = await openBrowserFilePicker();
    if (files.length === 0) return [];
    const { attached, errors } = await uploadAttachments(files, {
      sessionId,
      upload: (input) => api.uploadAttachment!(input),
    });
    reportAttachmentFailures(errors);
    return attached.map((a) => a.path);
  };

  const api: ElectronAPI = {
    platform: 'web' as unknown as NodeJS.Platform,

    // No native window chrome in the browser mirror.
    setTitleBarOverlay: () => {},

    // Worktree creation shells out on the HOST; the web mirror can't. The
    // spawn dialog hides the toggle when these report not-a-repo/unavailable.
    worktreeInfo: () => Promise.resolve({ isRepo: false }),
    worktreeCreate: () =>
      Promise.resolve({ ok: false, error: 'not available over the hub bridge' }),
    worktreeRemove: () => Promise.resolve({ ok: false, skipped: true }),

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
    // The result carries more than the id: `fullAccess` is what the session
    // ACTUALLY runs with (the hub stamps it on every spawn, honoring or
    // clamping the request by the CALLER's token scope), and
    // `escalationScrubbed` names anything asked for and refused. Both are
    // recorded against the session before the id is handed back, so the pane
    // that is about to mount already has the truth — the caller's signature is
    // unchanged and every other consumer reads it off the snapshot, where the
    // pills already look.
    spawnClaude: (opts) =>
      client
        .call<{
          sessionId: string;
          fullAccess?: boolean;
          escalationScrubbed?: string[];
        }>('agents.spawn', opts)
        .then((r) => {
          // A hub that predates the stamp sends no `fullAccess`. Absent is NOT
          // false — inventing "ask mode" there is the very bug this fixes — so
          // record nothing and let the pill say Unknown.
          if (typeof r.fullAccess === 'boolean') {
            noteLaunch(r.sessionId, {
              permissionMode: launchPermissionMode(
                opts.provider,
                r.fullAccess,
                opts.permissionMode,
              ),
              fullAccess: r.fullAccess,
              escalationScrubbed: r.escalationScrubbed,
            });
          }
          return r.sessionId;
        }),
    // The bus has no config-changed event yet, so the web mirror keeps the
    // old behaviour: its snapshot refreshes on its own saves and on reload.
    onConfigChanged: () => () => {},
    claudeListModels: () => client.call('claude.listModels', {}),
    // Auto-titling runs a local headless `claude --print` in the desktop main
    // process; over the bus there is no such capability. Null = leave the name
    // alone — the desktop client titles the agent and the layout syncs it here.
    agentSuggestTitle: async () => null,
    // Workflow-run transcripts still live in local Claude artifact files, but
    // plain provider-native subagent rows (runId null) can be read through
    // claudemon and folded client-side.
    workflowAgentTranscript: async (sessionId, runId, agentId) => {
      const conv = await readProviderSubagentConversation(sessionId, runId, agentId);
      if (!conv) return null;
      return conv
        .map((turn) => ({ role: turn.role, text: transcriptLineText(turn) }))
        .filter((turn) => turn.text.length > 0);
    },
    workflowAgentConversation: (sessionId, runId, agentId) =>
      readProviderSubagentConversation(sessionId, runId, agentId),
    // Live per-provider discovery over the bus (providers.* capabilities): the
    // managed provider's model catalog and PATH-detection status, so the web
    // Spawn dialog matches the desktop instead of falling back to free-text.
    providerListModels: (provider, cwd) => client.call('providers.listModels', { provider, cwd }),
    providerCheckAll: () => client.call('providers.checkAll', {}),
    // Keep-warm heartbeats live in the desktop's claudemon; not exposed over
    // the hub bus (settings-only surface), so the web client shows none.
    keepWarmHeartbeats: async () => [],
    claudeMessage: (sessionId, text) =>
      client.call<{ ok: boolean; mode?: string }>(qualify(sessionId, 'agents.sendMessage'), {
        sessionId,
        text,
      }),
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
    claudeSetModel: (sessionId, model, effort, modelIdentity, contextWindow) =>
      client.call<{
        ok: boolean;
        error?: string;
        model?: string;
        requestedSelection?: { model: string; contextWindow: number | null };
        queued?: boolean;
        disposition?: 'queued' | 'accepted';
      }>(qualify(sessionId, 'claude.setModel'), {
        sessionId,
        model,
        effort,
        modelIdentity,
        contextWindow,
      }),
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
      client
        .call<void>(qualify(sessionId, 'claude.approve'), { sessionId, decision, reason })
        .then(() => {}),
    claudeAnswer: (sessionId, payload) =>
      client
        .call<void>(qualify(sessionId, 'claude.answer'), { sessionId, ...payload })
        .then(() => {}),
    claudeResize: (sessionId, cols, rows) => {
      reprime(sessionId);
      return client.call<void>('sessions.terminalResize', { sessionId, cols, rows }).then(() => {});
    },
    claudeSignal: (sessionId, signal) =>
      client.call<void>(qualify(sessionId, 'claude.signal'), { sessionId, signal }).then(() => {}),
    claudeClose: (sessionId) =>
      client
        .call<void>(qualify(sessionId, 'claude.signal'), { sessionId, signal: 'SIGTERM' })
        .then(() => {}),
    attachClaude: (paneId, sessionId) => {
      viewerSessions.set(paneId, sessionId);
      return Promise.resolve(sessionId);
    },
    detachClaude: (paneId) => {
      const sessionId = viewerSessions.get(paneId);
      viewerSessions.delete(paneId);
      // The last watch pane on this session closed: stop the delta bytes (the
      // unsubscribe releases the hub's demand count for this client). The
      // session stays watched, so the poll tick resumes as the fallback if a
      // spawner pane still renders it — and the next getClaudeSession (any
      // pane activation) re-arms the push. Spawner panes have no detach; their
      // close SIGTERMs the session, and the ended row tears everything down.
      if (sessionId && ![...viewerSessions.values()].includes(sessionId)) {
        disarmPush(sessionId);
        pacePolling(sessionId);
      }
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
          const seeded = seedFull(hub ? { ...s, hub } : s);
          // A pane asked for this session by id — that is what "someone is
          // watching it" means here, so start (and prime) its transcript sync,
          // and subscribe to its delta feed. useClaudeSession re-calls this on
          // every pane activation, which is also what re-arms a push that a
          // pane close disarmed.
          if (!Array.isArray(seeded.conversation) && seeded.status !== 'ended') {
            watchedSessions.add(sessionId);
            void conversations.poke(sessionId);
            armPush(sessionId);
          }
          return withConversation(seeded);
        }),
    getAllClaudeSessions: () =>
      client
        .call<ClaudeSessionSnapshot[]>('sessions.snapshots', {})
        .then((list) => (list || []).map(foldSparse))
        // Federation: the LOCAL call answers with the local fleet only; the
        // peers' fleets are fetched over their links and arrive hub-stamped.
        .then(withPeerFleets),
    // The daemon's full resumable-session list, enriched host-side (history DB
    // names/cost + provider auto-titles). The rejection is DELIBERATELY not
    // swallowed: `sessions.recent` has no provider on a headless hub, and
    // answering [] made the Sessions pane say "No past sessions — everything is
    // already in your workspace", which is a confident wrong answer about
    // someone's history rather than an admission that we cannot see it.
    // useRecentSessions turns the rejection into an honest empty state.
    listRecentAgentSessions: () => client.call<RecentAgentSession[]>('sessions.recent', {}),
    // Null = "can't tell" — the web client never reconciles/auto-respawns
    // agents against the daemon; the desktop owns that.
    listLiveClaudeSessionIds: () => Promise.resolve(null),
    // Federation: served by the hub-local `federation.peers` method (a browser
    // can't read peers.json). [] on an older hub or federation off.
    // Federation: remote conversation over the qualified call — the web build
    // can serve this directly off the bus. Local sessions read through the
    // normal snapshot flow, so answer null for them (matches the desktop).
    federationConversation: (sessionId) => {
      if (!sessionHub.has(sessionId)) return Promise.resolve(null);
      // The pane's `sinceSeq` argument is accepted and ignored, exactly as
      // main's federation bridge documents: the fold owns its own seq
      // tracking. The ITEMS are the point — before, the web build fetched them
      // and dropped them on the floor, so a peer's session rendered as empty a
      // chat as a headless local one.
      watchedSessions.add(sessionId);
      // A peer's session that is mid-turn when the pane opens gets its own
      // clock straight away, rather than waiting for the next stamped push.
      pacePolling(sessionId);
      return conversations
        .poke(sessionId)
        .then((res: ConversationSnapWire | null) =>
          res && typeof res.seq === 'number'
            ? { seq: res.seq, items: (res.items ?? []) as unknown[] }
            : null,
        );
    },
    // Peers are configured on the hub MACHINE (peers.json + hub restart);
    // the web mirror can see the fleet but not edit the links. Null tells the
    // settings UI to render read-only.
    federationPeersConfig: async () => null,
    federationSavePeersConfig: async () => ({
      ok: false,
      error: 'peers are configured on the hub machine, not from the web client',
    }),
    federationPeers: () =>
      client
        .call<Array<{ name: string; connected: boolean; lastSeen?: number }>>(
          'federation.peers',
          {},
        )
        .then((peers) => peers ?? [])
        .catch(() => []),
    onClaudeSessionUpdate: (callback) => {
      sessionUpdateCbs.add(callback);
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
          // A sparse row that stayed conversation-less (no rich desktop row to
          // overlay — the headless case) gets the transcript we fetched, and
          // this tick is what re-arms the fetch: state changed, so there is
          // probably a new turn to pull.
          const shown = withConversation(merged);
          if (watchedSessions.has(shown.sessionId) && !Array.isArray(merged.conversation)) {
            void conversations.poke(shown.sessionId);
          }
          callback(shown.sessionId, shown);
          return;
        }
        const merged = foldConversation(snap);
        if (merged) callback(merged.sessionId, withConversation(merged));
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
        sessionUpdateCbs.delete(callback);
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
    // Hub jobs — trusted-only hub-local RPCs, so these work for a
    // full-control pairing and error cleanly for view/triage tokens (the
    // settings section feature-detects by the first list failing).
    // Remote worker nodes. `nodes.list` is in the bus's VIEW tier, so any
    // token can read the state — but the hub only REGISTERS the method when a
    // nodes.json exists, so "no provider" is the feature-absent signal and is
    // normalised to null here (identical to main's handler). Anything else
    // rethrows: a broken hub must not render as a hub with no nodes.
    //
    // `canWake` is this connection's OWN tier off the hello frame. nodes.wake
    // and nodes.sleep are both host-authority only, so a view/triage phone gets
    // the state and NEITHER button — never one the bus is certain to refuse.
    nodesList: async () => {
      try {
        const nodes = await client.call<unknown>('nodes.list', {});
        return {
          nodes: Array.isArray(nodes) ? nodes : [],
          canWake: client.can('nodes.wake'),
        };
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (/no provider for nodes\.list\b/.test(msg)) return null;
        throw err;
      }
    },
    nodesWake: async (id: string) => {
      try {
        const node = await client.call<unknown>('nodes.wake', { id });
        return { ok: true, node };
      } catch (err) {
        return { ok: false, error: err instanceof Error ? err.message : String(err) };
      }
    },
    // The stop half, over the same bus and behind the same hub-side gate. Only
    // an id goes out: the machine, the credential, the signal and the drain
    // window are all the hub's own.
    nodesSleep: async (id: string) => {
      try {
        const node = await client.call<unknown>('nodes.sleep', { id });
        return { ok: true, node };
      } catch (err) {
        return { ok: false, error: err instanceof Error ? err.message : String(err) };
      }
    },
    jobsList: () => client.call('jobs.list', {}),
    jobsUpsert: (job) => client.call('jobs.upsert', job),
    jobsRemove: (id) => client.call('jobs.remove', { id }),
    jobsRun: (id) => client.call('jobs.run', { id }),
    jobsHistory: (id) => client.call('jobs.history', { id }),
    layoutGet: () => client.call('layout.get', {}),
    layoutSet: (data) => client.call('layout.set', { data }),
    onLayoutChanged: (callback) =>
      client.subscribe('layout.changed', (ev) =>
        callback(ev.data as { version: number; data: unknown }),
      ),
    // The brain (headless provider) has no renderer to push IPC.
    // FACADE_OPEN_TERMINAL to, so it publishes the identical payload as
    // facade.openTerminal on the bus instead (visibleterm.go). The topic is
    // TopicGuardedBy terminals.open (eventtopics.go) — the hub itself refuses
    // this subscription for a connection that doesn't hold that capability,
    // so no client-side gating is needed here.
    onFacadeOpenTerminal: (callback) =>
      client.subscribe('facade.openTerminal', (ev) =>
        callback(
          (ev.data ?? {}) as {
            cwd?: string;
            command?: string;
            label?: string;
            parentSessionId?: string;
          },
        ),
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
    // The plugin catalogue. Not host-owned: main's IPC handler is itself a thin
    // proxy over GET /plugins + GET /plugins/tokens, and this client holds the
    // same bearer token, so it asks the hub directly. Without it `/app` (and the
    // desktop's remote-client mode, which is this backend) knew of no plugins at
    // all — no palette entries, no pane menu, no widgets — and a plugin pane
    // could only be reached by restoring a layout some desktop client wrote.
    //
    // It also stamps the two bases the renderer must never guess (types/plugin.ts):
    // where this client reaches the hub, and — only when the hub is this very
    // machine — where it reaches a sidecar's own loopback port. A browser
    // elsewhere gets no serverBase, because `127.0.0.1:<port>` from there is the
    // VIEWER's computer.
    listHubPlugins: async () => {
      try {
        const res = await fetch(`${hubHttpBase}/plugins`, {
          headers: hubAuth,
          signal: AbortSignal.timeout(5000),
        });
        // null, never []: usePlugins reads null as "hub not answering" and
        // retries with backoff, and [] as "no plugins installed".
        if (!res.ok) return null;
        // The hub serves either the full manifest (this client is authorized) or
        // the PUBLIC projection, which withholds `ui`/`server` on purpose — so
        // treat every field as optional and stamp only what is there.
        const plugins = (await res.json()) as PluginManifest[];
        // Per-plugin bus tokens ride a separate token-guarded route (never the
        // public /plugins projection). Best-effort, exactly as main does it: no
        // tokens → the guests can't call capabilities, but the list still renders.
        try {
          const tokRes = await fetch(`${hubHttpBase}/plugins/tokens`, {
            headers: hubAuth,
            signal: AbortSignal.timeout(5000),
          });
          if (tokRes.ok) {
            const tokens = (await tokRes.json()) as Record<string, string>;
            for (const p of plugins) {
              if (tokens[p.id]) p.busToken = tokens[p.id];
            }
          }
        } catch {
          /* tokens unavailable — degrade to no capability calls from the guest */
        }
        for (const p of plugins) {
          if (p.ui) p.uiBase = hubOrigin;
          if (hubIsThisMachine && p.server?.port) {
            p.serverBase = `http://127.0.0.1:${p.server.port}`;
          }
        }
        return plugins;
      } catch {
        return null;
      }
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
    // The browser's own file picker, then the bytes over `files.upload` — the
    // path that comes back is on the machine RUNNING the agent, which is the
    // only kind of path an attachment prefix can mean. This used to be a
    // `window.prompt` asking the viewer to type paths "on the host": it looked
    // like a feature and attached files that existed on neither machine.
    //
    // Desktop (bus mode) never reaches this — `pickFiles` is HOST_ONLY, so the
    // native dialog and its real host paths still win there. Desktop
    // REMOTE-CLIENT mode does reach it, and must: its host is not the agent's.
    pickFiles: (_defaultPath?: string, opts?: { attachment?: boolean; sessionId?: string }) =>
      opts?.attachment ? pickAndUpload(opts.sessionId) : refuseHostFilePick(),
    // Land bytes from this client on the agent's machine. Qualified for
    // federation exactly like agents.sendMessage, so an attachment for a
    // session living on a peer hub is written by that peer's own hub — the
    // path in the prefix has to resolve where the agent will open it.
    uploadAttachment: ({ name, dataBase64, sessionId }) =>
      client.call<{ path: string; size: number }>(
        sessionId ? qualify(sessionId, 'files.upload') : 'files.upload',
        { name, dataBase64 },
        UPLOAD_TIMEOUT_MS,
      ),
    // A browser has no host path for a dropped file — the file is on the client
    // machine, the agent runs on the host. '' is not just a degraded answer, it
    // is the SIGNAL the composer keys its upload fallback off (see
    // lib/attachmentUpload): a platform check would be wrong in remote-client
    // mode, which reports the genuine host platform but has the wrong host.
    getPathForFile: () => '',
    // The host's clipboard is not the one the browser user pasted from, so
    // there is nothing to spill. null sends the paste handler down the upload
    // path with the bytes the browser itself gave it.
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
    // The host's default browser is unreachable from a tab — but the viewer is
    // sitting in one. Leaving this undefined made every notification carrying a
    // url a dead click (NotificationsContext feature-detects it). The url is
    // already scheme-checked at the notification chokepoint (notificationStore
    // safeUrl), so http(s) is all that reaches here.
    openExternalUrl: async (url: string) => {
      window.open(url, '_blank', 'noopener,noreferrer');
      return { ok: true as const };
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
