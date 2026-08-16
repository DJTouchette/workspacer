/**
 * Desktop-side hub federation awareness.
 *
 * The local hub links to peer hubs (services/hub/internal/federation),
 * republishes their `agent.*` / `workflow.*` events locally with the peer name
 * stamped on the envelope (`hub`), and publishes `hub.peer.connected` /
 * `hub.peer.disconnected` lifecycle events. This bridge is main's consumer of
 * that feed:
 *
 *   - hub-stamped `agent.snapshot`      → upsert a REMOTE session into
 *     claudeSessionStore, flowing to the renderer over the normal
 *     claude-session:update channel (the renderer renders any snapshot with
 *     `hub` set as a remote card);
 *   - hub-stamped `agent.state_changed` → light-touch ambient update;
 *   - hub-stamped `workflow.*`          → ignored for v1 (the snapshot feed
 *     carries workflow summaries);
 *   - `hub.peer.connected`              → mark the peer up and SEED: call the
 *     peer's own `sessions.snapshots` capability through the qualified method
 *     name (`hub:<peer>/sessions.snapshots`) and replace that hub's sessions
 *     wholesale;
 *   - `hub.peer.disconnected`           → tombstone (hubOffline) that hub's
 *     sessions and record lastSeen.
 *
 * Remote snapshots come in two shapes and both are ingested: rich rows from a
 * peer desktop (bounded conversation window) and `sparse` rows from a peer
 * running only the headless brain (`workspacer serve` — state/attention only,
 * no conversation). The full transcript for either arrives on demand through
 * the `federation:conversation` IPC registered here: the renderer pokes it for
 * a remote pane, main calls `hub:<peer>/sessions.conversation` and folds the
 * items into the store (fetchRemoteConversation), and the normal snapshot push
 * carries the result everywhere.
 *
 * The peers map backs the `federation:peers` IPC. There is no way to ASK the
 * hub for already-connected peers (lifecycle events fire on transitions only),
 * so a peer whose link was already up when this process subscribed is
 * discovered implicitly: the first hub-stamped event from an unknown (or
 * believed-down) peer is treated as its connected transition and triggers the
 * same seed.
 */

import { ipcMain } from 'electron';
import { IPC } from '../shared/ipcChannels';
import { subscribeHubEvents, callHub, type HubEvent } from './hubClient';
import { claudeSessionStore, type RemoteSnapshotWire } from './claudeSessionStore';
import type { ConversationItemWire } from './sessionStore/conversationApplier';
import type { FederationPeerInfo } from '../shared/ipcTypes';

const peers = new Map<string, FederationPeerInfo>();
let unsubscribe: (() => void) | null = null;

/** The peer's `sessions.conversation` result (claudemon's shape). */
interface RemoteConversationSnap {
  seq: number;
  items: ConversationItemWire[];
}

function isConversationSnap(v: unknown): v is RemoteConversationSnap {
  return (
    !!v &&
    typeof (v as RemoteConversationSnap).seq === 'number' &&
    Array.isArray((v as RemoteConversationSnap).items)
  );
}

// Single flight per session: ClaudePane pokes on every activity bump, and two
// overlapping fetches folding interleaved batches is exactly the duplicate-turn
// hazard the applier's dedup shouldn't be leaned on for.
const conversationFetches = new Map<string, Promise<RemoteConversationSnap | null>>();

/**
 * Fetch a REMOTE session's conversation over its federation link and fold it
 * into the session store, so the normal snapshot push carries the full
 * transcript to every consumer (the desktop otherwise renders only the
 * compacted window the peer's agent.snapshot events hold — /m and the TUI
 * already fetch full history this way; this closes the desktop gap).
 *
 * Main's own folded-seq tracking (claudeSessionStore.remoteConversationSeq) is
 * authoritative for `sinceSeq` — the renderer's copy can be stale across a
 * main-side rebuild or reseed. First fetch is full; later ones incremental.
 * A seq that moved BACKWARD means the peer's claudemon restarted its stream —
 * our folded history may describe a conversation that no longer exists, so
 * refetch from the top and rebuild.
 *
 * Returns null for local/unknown sessions, tombstoned (hubOffline) sessions —
 * a down link must stop the polling, reconnect's reseed resumes it — and on
 * any link error.
 */
export function fetchRemoteConversation(sessionId: string): Promise<RemoteConversationSnap | null> {
  if (!sessionId || typeof sessionId !== 'string') return Promise.resolve(null);
  const snap = claudeSessionStore.getSnapshot(sessionId);
  const hub = snap?.hub;
  if (!hub || snap.hubOffline) return Promise.resolve(null);
  const inflight = conversationFetches.get(sessionId);
  if (inflight) return inflight;
  const fetch = (async (): Promise<RemoteConversationSnap | null> => {
    try {
      const since = claudeSessionStore.remoteConversationSeq(sessionId);
      let res = await callHub<RemoteConversationSnap>(
        `hub:${hub}/sessions.conversation`,
        since !== undefined ? { sessionId, sinceSeq: since } : { sessionId },
      );
      if (!isConversationSnap(res)) return null;
      let rebuild = since === undefined;
      if (since !== undefined && res.seq < since) {
        res = await callHub<RemoteConversationSnap>(`hub:${hub}/sessions.conversation`, {
          sessionId,
        });
        if (!isConversationSnap(res)) return null;
        rebuild = true;
      }
      claudeSessionStore.applyRemoteConversation(hub, sessionId, res.seq, res.items, rebuild);
      return res;
    } catch (err) {
      console.warn(
        `[federation] conversation fetch for ${sessionId} (hub "${hub}") failed:`,
        err instanceof Error ? err.message : err,
      );
      return null;
    } finally {
      conversationFetches.delete(sessionId);
    }
  })();
  conversationFetches.set(sessionId, fetch);
  return fetch;
}

/** Peers main has observed, for the federation:peers IPC. */
export function listFederationPeers(): FederationPeerInfo[] {
  return Array.from(peers.values()).map((p) => ({ ...p }));
}

function markConnected(name: string): void {
  const p = peers.get(name) ?? { name, connected: false };
  p.connected = true;
  p.lastSeen = Date.now();
  peers.set(name, p);
}

/** Pull the peer's full session list and replace that hub's remote sessions. */
async function seedPeer(name: string): Promise<void> {
  try {
    const snaps = await callHub<RemoteSnapshotWire[]>(`hub:${name}/sessions.snapshots`, {});
    claudeSessionStore.reseedRemoteSessions(name, Array.isArray(snaps) ? snaps : []);
  } catch (err) {
    // The link can flap between the connected event and the call; the next
    // connected transition (or stamped event) retries. Sessions we already
    // hold stay tombstoned rather than being wiped on a failed refresh.
    console.warn(
      `[federation] seeding sessions from peer "${name}" failed:`,
      err instanceof Error ? err.message : err,
    );
  }
}

function handleEvent(ev: HubEvent): void {
  if (ev.type === 'hub.peer.connected' || ev.type === 'hub.peer.disconnected') {
    const data = (ev.data ?? {}) as { peer?: string; lastSeen?: string };
    const name = data.peer;
    if (!name || typeof name !== 'string') return;
    if (ev.type === 'hub.peer.connected') {
      markConnected(name);
      void seedPeer(name);
    } else {
      const p = peers.get(name) ?? { name, connected: false };
      p.connected = false;
      // The hub reports lastSeen as RFC3339; the IPC contract carries unix ms.
      const t = data.lastSeen ? Date.parse(data.lastSeen) : NaN;
      p.lastSeen = Number.isFinite(t) ? t : (p.lastSeen ?? Date.now());
      peers.set(name, p);
      claudeSessionStore.markHubPeerOffline(name);
    }
    return;
  }

  const hub = ev.hub;
  if (!hub) return; // local event — the store's local machinery owns it

  // Implicit peer discovery (see module comment): a stamped event proves the
  // link is up right now, even if we never saw its connected transition.
  const known = peers.get(hub);
  if (!known || !known.connected) {
    markConnected(hub);
    void seedPeer(hub);
  } else {
    known.lastSeen = Date.now();
  }

  if (ev.type === 'agent.snapshot') {
    claudeSessionStore.upsertRemoteSession(hub, (ev.data ?? {}) as RemoteSnapshotWire);
  } else if (ev.type === 'agent.state_changed') {
    const d = (ev.data ?? {}) as { sessionId?: string; mode?: string };
    if (d.sessionId && d.mode) claudeSessionStore.applyRemoteStateChange(hub, d.sessionId, d.mode);
  }
  // workflow.*: ignored for v1.
}

/** Start listening (idempotent). Call after startHubClient(). */
export function startFederationBridge(): void {
  if (unsubscribe) return;
  unsubscribe = subscribeHubEvents(handleEvent);
  // Renderer contract (preload's federationConversation): the sinceSeq argument
  // is accepted but unused — main's own folded-seq tracking decides full vs
  // incremental (see fetchRemoteConversation).
  ipcMain.handle(IPC.FEDERATION_CONVERSATION, (_event, sessionId: string) =>
    fetchRemoteConversation(sessionId),
  );
}

/** Stop and forget peer state (tests / shutdown). */
export function stopFederationBridge(): void {
  unsubscribe?.();
  unsubscribe = null;
  peers.clear();
  conversationFetches.clear();
  ipcMain.removeHandler(IPC.FEDERATION_CONVERSATION);
}
