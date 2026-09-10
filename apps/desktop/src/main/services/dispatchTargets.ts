/** Explicit worker-target discovery. Pairing credentials stay in main; the
 * remote host supplies protocol support, provider login status and repository
 * choices. Enabling a paired worker target never switches the desktop backend.
 * Existing federation peers retain their separate opt-in dispatch flag. */
import { getPairedWorkerTarget } from './remoteServer';
import { pairedWorkerConnection } from './pairedWorkerConnection';
import { callHub } from './hubClient';
import { listFederationPeers } from './federationBridge';
import { readRedactedPeers } from './federationPeersConfig';

/** How long a target's readiness probe may take. Deliberately shorter than the
 *  federation forwarder's own 25s budget: this is a menu being rendered for an
 *  agent, and one unreachable machine must not stall the whole answer. */
const PROBE_TIMEOUT_MS = 8000;

export interface DispatchProviderReadiness {
  provider: string;
  found: boolean;
  /** null = the TARGET cannot read this harness's login state (copilot's token
   *  is in the OS credential store). Never collapsed to false: "not signed in"
   *  and "cannot tell" call for different decisions. */
  authenticated: boolean | null;
  note: string;
}

export interface DispatchCwdChoice {
  path: string;
  source: string;
  git: boolean;
}

export interface DispatchTarget {
  /** The peers.json name. This is the string a dispatch passes as `target`, and
   *  the string our federation link stamps on the callbacks that come home. */
  name: string;
  /** Host only, for display. Never the token, never the full URL. */
  host: string;
  /** Is the outbound link up right now? */
  connected: boolean;
  lastSeen?: number;
  /** Is a credential configured for this peer at all? A linked machine with no
   *  token cannot be reached, and the failure is otherwise unexplained. */
  hasToken: boolean;
  /** Can this target actually execute dispatched work AND report back? False
   *  for an unreachable machine, one running an older workspacer with no
   *  remote-dispatch support, and one registered in catalog scope. */
  ready: boolean;
  /** One sentence saying why `ready` is what it is. Always present. */
  readiness: string;
  /** The target's remote-dispatch protocol number, when it answered. */
  protocol?: number;
  /** Measured ON THE TARGET. Empty when the probe failed. */
  providers: DispatchProviderReadiness[];
  /** REMOTE absolute paths. Pass one verbatim; never translate a local path. */
  cwds: DispatchCwdChoice[];
}

export interface DispatchTargetsAnswer {
  targets: DispatchTarget[];
  /** Linked machines that are NOT enabled as worker targets, by name, so an
   *  operator reading an empty list knows the difference between "nothing is
   *  linked" and "nothing is ticked". */
  linkedButNotEnabled: string[];
  note: string;
}

/** The shape fleet.dispatchCapabilities answers with (cmd/brain/remotedispatch.go). */
interface CapabilitiesReply {
  protocol?: number;
  executes?: boolean;
  scope?: string;
  unsupportedReason?: string;
  providers?: DispatchProviderReadiness[];
  cwds?: DispatchCwdChoice[];
}

/** The protocol this desktop speaks. Twin: bus.DispatchProtocol. */
export const DISPATCH_PROTOCOL = 2;

/** Reduce a peer URL to a host for display. Never throws: an unparseable URL
 *  degrades to the raw string with any credentials-looking prefix removed. */
export function peerHost(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url.replace(/^[a-z]+:\/\/([^@/]*@)?/i, '').split('/')[0] ?? url;
  }
}

function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`timed out after ${ms}ms`)), ms);
    t.unref?.();
    p.then(
      (v) => {
        clearTimeout(t);
        resolve(v);
      },
      (e) => {
        clearTimeout(t);
        reject(e);
      },
    );
  });
}

/**
 * The probe, injectable so the whole answer is testable without a hub.
 * Rejecting is normal (peer down, method unknown on an older build) and is
 * turned into an honest not-ready row rather than an exception.
 */
export type CapabilitiesProbe = (peer: string) => Promise<CapabilitiesReply>;

const defaultProbe: CapabilitiesProbe = (peer) =>
  withTimeout(
    peer === 'paired'
      ? pairedWorkerConnection.call<CapabilitiesReply>('fleet.dispatchCapabilities')
      : callHub<CapabilitiesReply>(`hub:${peer}/fleet.dispatchCapabilities`, {}),
    PROBE_TIMEOUT_MS,
  );

/**
 * Build the target list. Probes run in parallel — one slow machine costs its own
 * timeout, not the sum.
 */
export async function listDispatchTargets(
  probe: CapabilitiesProbe = defaultProbe,
  peersFn = readRedactedPeers,
  liveFn = listFederationPeers,
): Promise<DispatchTargetsAnswer> {
  const pair = getPairedWorkerTarget();
  const configured = peersFn().filter((p) => !pair || p.name !== 'paired');
  if (pair)
    configured.unshift({
      name: 'paired',
      url: pair.busUrl,
      hasToken: !!pair.token,
      dispatch: true,
    });
  const live = new Map(liveFn().map((p) => [p.name, p]));
  const enabled = configured.filter((p) => p.dispatch);
  const linkedButNotEnabled = configured.filter((p) => !p.dispatch).map((p) => p.name);

  const targets = await Promise.all(
    enabled.map(async (peer): Promise<DispatchTarget> => {
      const info = live.get(peer.name);
      const base: DispatchTarget = {
        name: peer.name,
        host: peerHost(peer.url),
        connected: info?.connected === true,
        ...(info?.lastSeen ? { lastSeen: info.lastSeen } : {}),
        hasToken: peer.hasToken,
        ready: false,
        readiness: '',
        providers: [],
        cwds: [],
      };
      if (!pair || peer.name !== 'paired') {
        return {
          ...base,
          readiness:
            'Legacy federation link: use Connect to Server → Workers only for a local-manager execution target. This link is not a paired dispatch route.',
        };
      }
      if (!peer.hasToken) {
        return {
          ...base,
          readiness:
            'No pairing credential is configured. Set up Workers only in Connect to Server.',
        };
      }
      let reply: CapabilitiesReply;
      try {
        reply = await probe(peer.name);
      } catch (err) {
        // Unreachable, or too old to know the method. Both are "not ready", and
        // the message says which so the operator is not left guessing between a
        // network problem and a version problem. UNKNOWN IS NEVER SUCCESS.
        return {
          ...base,
          readiness:
            `unreachable or unsupported: ${err instanceof Error ? err.message : String(err)}. ` +
            `A machine that does not answer fleet.dispatchCapabilities is either offline or running a workspacer older than remote worker dispatch.`,
        };
      }
      const protocol = typeof reply.protocol === 'number' ? reply.protocol : undefined;
      if (protocol !== DISPATCH_PROTOCOL) {
        return {
          ...base,
          ...(protocol !== undefined ? { protocol } : {}),
          readiness:
            protocol === undefined
              ? 'that machine answered without a remote-dispatch protocol number — it is running a workspacer older than this feature'
              : `remote-dispatch protocol mismatch: this machine speaks ${DISPATCH_PROTOCOL}, that one speaks ${protocol}. Upgrade the older install.`,
        };
      }
      const executes = reply.executes === true;
      if (peer.name === 'paired') base.connected = true;
      return {
        ...base,
        protocol,
        providers: Array.isArray(reply.providers) ? reply.providers : [],
        cwds: Array.isArray(reply.cwds) ? reply.cwds : [],
        ready: executes && base.connected,
        readiness: !executes
          ? (reply.unsupportedReason ??
            'that machine cannot execute dispatched work — it is not running a full-scope workspacer server')
          : base.connected
            ? 'ready: the link is up and that machine can execute dispatched work and report back'
            : 'that machine supports dispatch but the link is down right now — its last known state is shown',
      };
    }),
  );

  targets.sort((a, b) => Number(b.ready) - Number(a.ready) || a.name.localeCompare(b.name));
  return {
    targets,
    linkedButNotEnabled,
    note: targets.length
      ? 'For the ready paired target, use spawn_agent with executionTarget="paired", cwd=the LOCAL task project, remoteCwd=one returned REMOTE cwd, and your own parentSessionId. Use select_dispatch_model with that remote cwd and an authenticated remote provider. Unknown authentication is unavailable. Do not use hub="paired"; this route is owned by the local desktop.'
      : linkedButNotEnabled.length
        ? 'Federation links are present, but no paired worker target is enabled. Use Connect to Server → Workers only to keep this manager local.'
        : 'No paired worker target is enabled. Use Connect to Server → Workers only; ordinary spawns continue to run locally.',
  };
}
