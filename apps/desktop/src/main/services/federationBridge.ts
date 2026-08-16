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
 * The peers map backs the `federation:peers` IPC. There is no way to ASK the
 * hub for already-connected peers (lifecycle events fire on transitions only),
 * so a peer whose link was already up when this process subscribed is
 * discovered implicitly: the first hub-stamped event from an unknown (or
 * believed-down) peer is treated as its connected transition and triggers the
 * same seed.
 */

import { subscribeHubEvents, callHub, type HubEvent } from './hubClient';
import { claudeSessionStore, type RemoteSnapshotWire } from './claudeSessionStore';
import type { FederationPeerInfo } from '../shared/ipcTypes';

const peers = new Map<string, FederationPeerInfo>();
let unsubscribe: (() => void) | null = null;

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
}

/** Stop and forget peer state (tests / shutdown). */
export function stopFederationBridge(): void {
  unsubscribe?.();
  unsubscribe = null;
  peers.clear();
}
