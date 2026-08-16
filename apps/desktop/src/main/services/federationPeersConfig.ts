/**
 * Federation peers editor — the desktop side of `~/.config/workspacer/peers.json`.
 *
 * The hub (services/hub/internal/federation/federation.go, LoadPeersFile) reads
 * this file at startup to dial outbound links to peer hubs. Until now it was
 * hand-edited; this service backs the Remote Control dialog's "Linked machines"
 * UI with two IPC handlers:
 *
 * - FEDERATION_PEERS_CONFIG    → the configured peers with tokens REDACTED
 *   (name/url/hasToken only — a bearer token for another machine must never
 *   round-trip through the renderer).
 * - FEDERATION_SAVE_PEERS_CONFIG → replace peers.json wholesale. An entry whose
 *   `token` is undefined KEEPS the stored token for that peer name (that is how
 *   the renderer re-sends existing rows without ever seeing their secrets).
 *   After a successful write the hub is restarted so the new peer set takes
 *   effect (links are dialed at hub startup; the desktop's hub client and the
 *   federation bridge auto-reconnect on their own).
 *
 * Validation mirrors the Go loader exactly — name `[A-Za-z0-9_-]+`, url ws://
 * or wss:// — so a file this service writes is always one the hub will accept
 * (a bad peers.json makes the hub fail loudly by design). The file carries
 * bearer tokens, so it is written atomically with mode 0o600, same as
 * tokens.json (remoteTokens.ts).
 */
import * as fs from 'fs';
import * as path from 'path';
import { ipcMain } from 'electron';
import { getConfigDir } from './configService';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { startHub, stopHub } from './hubDaemon';
import { getRemoteServer } from './remoteServer';
import { IPC } from '../shared/ipcChannels';

/** One stored peer entry, as the hub's LoadPeersFile reads it. */
export interface FederationPeerEntry {
  name: string;
  url: string;
  token?: string;
}

/** What the renderer sees: the token never crosses, only its presence. */
export interface RedactedFederationPeer {
  name: string;
  url: string;
  hasToken: boolean;
}

export type SavePeersResult = { ok: true } | { ok: false; error: string };

/** Twin of federation.go's validPeerName — the name is interpolated into
 *  `hub:<name>/<method>` call routing, so it stays this strict on both sides. */
const PEER_NAME_RE = /^[A-Za-z0-9_-]+$/;

/** Same path the hub reads: <config>/workspacer/peers.json (DefaultPeersPath). */
export function peersConfigPath(): string {
  return path.join(getConfigDir(), 'peers.json');
}

/** Read the stored peers. Missing file = no peers (like the Go loader); a
 *  corrupt file reads as empty here — the UI can then repair it with a save,
 *  while the hub keeps failing loudly on its own read. */
function readStoredPeers(): FederationPeerEntry[] {
  let raw: string;
  try {
    raw = fs.readFileSync(peersConfigPath(), 'utf-8');
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return [];
    throw err;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    console.warn('[federation] peers.json is not valid JSON — treating as empty');
    return [];
  }
  if (!Array.isArray(parsed)) {
    console.warn('[federation] peers.json is not a JSON array — treating as empty');
    return [];
  }
  const out: FederationPeerEntry[] = [];
  for (const item of parsed) {
    if (!item || typeof item !== 'object') continue;
    const r = item as Record<string, unknown>;
    if (typeof r.name !== 'string' || typeof r.url !== 'string') continue;
    const entry: FederationPeerEntry = { name: r.name.trim(), url: r.url.trim() };
    if (typeof r.token === 'string' && r.token) entry.token = r.token;
    out.push(entry);
  }
  return out;
}

/** The read side of the IPC contract: configured peers, tokens redacted. */
export function readRedactedPeers(): RedactedFederationPeer[] {
  return readStoredPeers().map((p) => ({ name: p.name, url: p.url, hasToken: !!p.token }));
}

/**
 * Validate + normalize renderer input. Mirrors LoadPeersFile's checks (and its
 * wording, so the error reads the same whichever side catches it), plus a
 * duplicate-name check the merge semantics depend on: keep-token is keyed by
 * name, so two rows with one name would be ambiguous.
 */
function normalizePeers(
  peers: unknown,
): { ok: true; peers: Array<{ name: string; url: string; token?: string }> } | { ok: false; error: string } {
  if (!Array.isArray(peers)) return { ok: false, error: 'peers must be an array' };
  const out: Array<{ name: string; url: string; token?: string }> = [];
  const seen = new Set<string>();
  for (const item of peers) {
    if (!item || typeof item !== 'object') {
      return { ok: false, error: 'peer entries need name and url' };
    }
    const r = item as Record<string, unknown>;
    const name = typeof r.name === 'string' ? r.name.trim() : '';
    const url = typeof r.url === 'string' ? r.url.trim() : '';
    if (!name || !url) return { ok: false, error: 'peer entries need name and url' };
    if (!PEER_NAME_RE.test(name)) {
      return { ok: false, error: `peer name "${name}": use letters, digits, - or _` };
    }
    if (!url.startsWith('ws://') && !url.startsWith('wss://')) {
      return { ok: false, error: `peer "${name}": url must be ws:// or wss://` };
    }
    if (seen.has(name)) return { ok: false, error: `duplicate peer name "${name}"` };
    seen.add(name);
    if (r.token !== undefined && typeof r.token !== 'string') {
      return { ok: false, error: `peer "${name}": token must be a string` };
    }
    const entry: { name: string; url: string; token?: string } = { name, url };
    if (typeof r.token === 'string') {
      // An explicit token replaces; an explicit empty string clears. Only
      // `undefined` (property left absent here) means "keep what's stored".
      entry.token = r.token.trim();
    }
    out.push(entry);
  }
  return { ok: true, peers: out };
}

/**
 * Replace peers.json wholesale, then restart the hub so the new links dial.
 * Keep-token semantics: an entry with `token === undefined` inherits the stored
 * token for that peer name (tokens never round-trip through the renderer).
 */
export async function savePeersConfig(peersInput: unknown): Promise<SavePeersResult> {
  const norm = normalizePeers(peersInput);
  if (!norm.ok) return norm;

  const storedByName = new Map(readStoredPeers().map((p) => [p.name, p]));
  const next: FederationPeerEntry[] = norm.peers.map((p) => {
    const entry: FederationPeerEntry = { name: p.name, url: p.url };
    // Property present (even '') = explicit from the caller; absent = keep the
    // stored token for this peer name.
    const token = 'token' in p ? p.token : storedByName.get(p.name)?.token;
    if (token) entry.token = token;
    return entry;
  });

  try {
    // Bearer tokens for other machines: atomic (temp + rename) and 0o600, the
    // same posture as tokens.json. A crash mid-save leaves the old file intact.
    atomicWriteFileSync(peersConfigPath(), `${JSON.stringify(next, null, 2)}\n`, { mode: 0o600 });
  } catch (err) {
    return { ok: false, error: `could not write peers.json: ${(err as Error).message}` };
  }

  // Remote-client mode: no local hub runs (and none should be spawned just to
  // pick up a peers file it isn't using). The file is saved for the next local
  // boot; nothing to restart now.
  if (getRemoteServer()) return { ok: true };

  // Peer links are dialed at hub startup, so a restart is what makes the new
  // set take effect. stopHub/startHub handle the adopted-hub case themselves
  // (an external `workspacer serve` is left running and re-adopted — it keeps
  // its own peer set until ITS owner restarts it). The desktop's hub client and
  // the federation bridge resubscribe on reconnect; nothing else needs a kick.
  try {
    await stopHub();
    await startHub();
  } catch (err) {
    return {
      ok: false,
      error: `peers saved, but the hub restart failed: ${(err as Error).message}`,
    };
  }
  return { ok: true };
}

/** Register the two IPC handlers. Called once from main/index.ts. */
export function startFederationPeersConfig(): void {
  ipcMain.handle(IPC.FEDERATION_PEERS_CONFIG, () => readRedactedPeers());
  ipcMain.handle(IPC.FEDERATION_SAVE_PEERS_CONFIG, (_e, peers: unknown) =>
    savePeersConfig(peers),
  );
}
