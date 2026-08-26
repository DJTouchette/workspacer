/**
 * Remote worker nodes (the hub's node registry) — the renderer's half.
 *
 * A "node" is a machine that can be OFF ON PURPOSE: `claudemon` + `brain --hub
 * …` running somewhere else (today, a Fly machine). The hub owns the registry
 * and the state machine; this module owns everything the UI needs to read one
 * honestly. The wire contract is `.workspacer/reports/2026-08-24-fly-wake-contract.md`.
 *
 * Two things here are load-bearing and neither is obvious:
 *
 *  1. **A permission check is NOT a feature check.** `nodes.list` sits in the
 *     bus's VIEW tier, so every token can call it — but the hub only REGISTERS
 *     the method when a `nodes.json` exists, which is to say never on an
 *     ordinary install. `no provider for nodes.list` therefore means "this hub
 *     has no remote nodes", not "something broke": see [[isNodeRegistryAbsent]].
 *     Every existing workspacer install must see no change whatsoever, so the
 *     absent case renders NOTHING rather than an empty state or a toast.
 *
 *  2. **`waking` is not `unreachable`, and neither is `stopping`.** A machine
 *     takes real seconds to boot and real seconds to drain, and a spinner that
 *     looks the same as a hang is what makes someone give up. The five states
 *     get five distinct presentations — tone, verb and copy — and the two
 *     transitional ones are the only ones that read as progress.
 *
 *  3. **Sleeping is not the harmless direction of waking.** A wake spends
 *     money and is undone by waiting; a stop ends the work in flight on a
 *     machine somebody may be using, so it is host-authority-only on the hub
 *     for a reason of its own, and its confirm copy names the WORK rather than
 *     the saving. See [[SLEEP_NOTE]].
 *
 * The states themselves are the hub's answer, never inferred here: running with
 * no provider is `unreachable` (not `available`), stopped-after-a-failed-wake is
 * `unreachable` (not `stopped`), and a node with no credential is never
 * `stopped` because the hub genuinely cannot tell.
 */

/** The five states. The distinction between the last four is the whole point.
 *  `stopping` is the sleep path's mirror of `waking`: a machine takes real
 *  seconds to drain, and rendering that as `unreachable` turns a deliberate act
 *  into a warning. It is also what stops a Connect button reappearing mid-drain
 *  and racing the stop. */
export type NodeState = 'available' | 'waking' | 'stopping' | 'stopped' | 'unreachable';

const NODE_STATES: readonly string[] = [
  'available',
  'waking',
  'stopping',
  'stopped',
  'unreachable',
];

/** The node's own exit record, read off its volume via `brain.info`. Absent
 *  means NOBODY KNOWS — the hub never fabricates an empty one. */
export interface NodeLastExit {
  /** `signal-TERM` / `signal-INT` = a deliberate stop. Anything else = a crash. */
  reason?: string;
  exitCode?: number;
  /** RFC3339, on the NODE's clock. Display only — never compute with it. */
  at?: string;
}

/** `nodes.NodeView` — the one payload `nodes.list`, `nodes.wake` and
 *  `node.state_changed` all carry. Timestamps are unix MILLISECONDS. */
export interface RemoteNodeView {
  id: string;
  label: string;
  state: NodeState;
  since?: number;
  lastSeen?: number;
  /** One human sentence, written to be read. Empty is normal for `available`. */
  detail?: string;
  wakeable: boolean;
  /** Consecutive failed wakes. Omitted when 0. A failed wake no longer leaves a
   *  machine billing: the hub stops one whose wake never produced a provider
   *  (`stopAfterFailedWake`), and its own `detail` says whether that stop
   *  worked. */
  wakeFailures?: number;
  lastExit?: NodeLastExit;
  /** THIS hub process issued the stop that put the machine to sleep. The only
   *  account of a stop that is readable while the machine is OFF — `lastExit`
   *  lives on the node's volume and only arrives one wake later. Absent means
   *  "this hub did not do it", NEVER "somebody else did": it is in-memory only,
   *  so a restarted hub honestly stops claiming it. */
  sleptByHub?: boolean;
  /** The hub's belief about the MACHINE's power, which is a different question
   *  from whether its provider answers — and the one a stop button needs.
   *  `unreachable` covers both "running and providing nothing" (a meter) and
   *  "off and broken" (nothing to switch off). READ THIS; never infer it from
   *  `detail`, whose text for an already-stopped machine says "…would not keep
   *  billing". Prose is not an API. */
  mayBeRunning?: boolean;
}

/** What a backend hands back for `nodesList()`. `null` (not `{nodes: []}`) is
 *  the feature-absent answer. `canWake` is the caller's own tier: the desktop
 *  holds the host token and always may; a view/triage phone never may. */
export interface RemoteNodesSnapshot {
  nodes: RemoteNodeView[];
  canWake: boolean;
}

/** The result of a sleep attempt. Same shape as a wake — a refusal is an
 *  ANSWER rendered on the row, not a rejection. */
export interface NodeSleepResult {
  ok: boolean;
  node?: RemoteNodeView;
  error?: string;
}

/** The result of a wake attempt. `ok:false` carries a rendered reason. */
export interface NodeWakeResult {
  ok: boolean;
  node?: RemoteNodeView;
  error?: string;
}

// ── feature detection ───────────────────────────────────────────────────────

/**
 * Is this "the hub has no node registry" rather than a failure?
 *
 * The bus router's own words for an unregistered method are `no provider for
 * <method>`, and that is the definitive signal — the same shape `workspacer
 * status` already uses to tell "no brain registered" from "bus unreachable".
 * Anything else (a dropped socket, a timeout, a malformed answer) is a real
 * error and must NOT be swallowed into "feature absent", or a broken hub would
 * render as a hub that simply has no nodes.
 */
export function isNodeRegistryAbsent(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : typeof err === 'string' ? err : '';
  return /no provider for nodes\.(list|wake|sleep)\b/.test(msg);
}

/** Is this the tier refusal? A view/triage token asking to spend money. */
export function isHostAuthorityRefusal(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : typeof err === 'string' ? err : '';
  return /requires host authority/.test(msg);
}

// ── wire hygiene ────────────────────────────────────────────────────────────

/** Coerce one wire row into a `RemoteNodeView`, or null if it is not one.
 *  An unknown `state` string is treated as `unreachable` rather than rendered
 *  raw: a state this client cannot presume to understand is, by definition,
 *  one it does not know how to get a working node out of. */
export function normalizeNode(raw: unknown): RemoteNodeView | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const id = typeof r.id === 'string' ? r.id : '';
  if (!id) return null;
  const state = (
    typeof r.state === 'string' && NODE_STATES.includes(r.state) ? r.state : 'unreachable'
  ) as NodeState;
  const node: RemoteNodeView = {
    id,
    label: typeof r.label === 'string' && r.label ? r.label : id,
    state,
    wakeable: r.wakeable === true,
  };
  if (typeof r.since === 'number' && Number.isFinite(r.since)) node.since = r.since;
  if (typeof r.lastSeen === 'number' && Number.isFinite(r.lastSeen)) node.lastSeen = r.lastSeen;
  if (typeof r.detail === 'string' && r.detail) node.detail = r.detail;
  if (typeof r.wakeFailures === 'number' && r.wakeFailures > 0) node.wakeFailures = r.wakeFailures;
  if (r.sleptByHub === true) node.sleptByHub = true;
  if (r.mayBeRunning === true) node.mayBeRunning = true;
  if (r.lastExit && typeof r.lastExit === 'object') {
    const e = r.lastExit as Record<string, unknown>;
    const exit: NodeLastExit = {};
    if (typeof e.reason === 'string' && e.reason) exit.reason = e.reason;
    if (typeof e.exitCode === 'number') exit.exitCode = e.exitCode;
    if (typeof e.at === 'string' && e.at) exit.at = e.at;
    if (exit.reason || exit.exitCode !== undefined || exit.at) node.lastExit = exit;
  }
  return node;
}

/** Coerce a whole `nodes.list` answer, dropping rows that aren't nodes. */
export function normalizeNodes(raw: unknown): RemoteNodeView[] {
  if (!Array.isArray(raw)) return [];
  const out: RemoteNodeView[] = [];
  for (const row of raw) {
    const n = normalizeNode(row);
    if (n) out.push(n);
  }
  return out;
}

/**
 * Patch a seeded list from one `node.state_changed` payload.
 *
 * Seed from `nodes.list`, then patch from the event — do NOT poll. The hub
 * publishes only on a real change, so every event is worth acting on. An event
 * for an id we don't hold appends (the registry is hand-edited and can grow
 * under a long-lived client); order is otherwise preserved, because
 * `nodes.list` returns registry order and a list that reshuffles under a
 * pointer is its own bug.
 */
export function applyNodeStateChange(
  nodes: RemoteNodeView[],
  incoming: RemoteNodeView,
): RemoteNodeView[] {
  const idx = nodes.findIndex((n) => n.id === incoming.id);
  if (idx < 0) return [...nodes, incoming];
  const next = nodes.slice();
  next[idx] = incoming;
  return next;
}

// ── presentation ────────────────────────────────────────────────────────────

/** Which status token a state paints with. `busy` is the working tone — the
 *  same one a thinking agent uses — which is exactly what a booting machine
 *  is, and it is what keeps `waking` from reading as a failure. */
export type NodeTone = 'success' | 'busy' | 'muted' | 'warning';

export interface NodePresentation {
  /** The state, in words, for the chip. */
  label: string;
  tone: NodeTone;
  /** Does this state animate? Only `waking` — motion means progress. */
  progress: boolean;
  /** One line under the chip when the hub sent no `detail` of its own. */
  fallbackDetail: string;
}

export const NODE_PRESENTATION: Record<NodeState, NodePresentation> = {
  available: {
    label: 'Connected',
    tone: 'success',
    progress: false,
    fallbackDetail: 'This machine is on the bus and answering.',
  },
  waking: {
    label: 'Starting…',
    tone: 'busy',
    progress: true,
    fallbackDetail: 'The machine is booting — usually ready in about 20 seconds.',
  },
  // The busy tone again, not a warning: shutting down on purpose is work in
  // progress, not a fault. This is the whole reason the state exists rather
  // than being folded into `unreachable`.
  stopping: {
    label: 'Shutting down…',
    tone: 'busy',
    progress: true,
    fallbackDetail: 'The machine is shutting down cleanly. It stops billing once it is off.',
  },
  stopped: {
    label: 'Asleep',
    tone: 'muted',
    progress: false,
    fallbackDetail: 'Switched off, and nothing is billing. Connecting will start it.',
  },
  unreachable: {
    label: "Can't reach",
    tone: 'warning',
    progress: false,
    fallbackDetail: "The hub can't get a working machine out of this one.",
  },
};

/** The CSS custom property a tone paints with. */
export function nodeToneVar(tone: NodeTone): string {
  switch (tone) {
    case 'success':
      return 'var(--wks-success)';
    case 'busy':
      return 'var(--wks-busy)';
    case 'warning':
      return 'var(--wks-warning)';
    default:
      return 'var(--wks-text-tertiary)';
  }
}

/** The sentence under a node's chip: the hub's own `detail` when it wrote one
 *  (it is written to be read by a person), else the state's fallback. */
export function nodeDetailLine(node: RemoteNodeView): string {
  return node.detail || NODE_PRESENTATION[node.state].fallbackDetail;
}

/**
 * The node telling you its last run crashed — the only notice anyone gets.
 *
 * Render whenever `lastExit` is present and its reason is not a `signal-`
 * (deliberate stop). A MISSING `lastExit` means nobody knows, NOT that it ended
 * cleanly, so this returns null in that case and says nothing rather than
 * reassuring anyone.
 */
export function nodeCrashNotice(node: RemoteNodeView): string | null {
  const exit = node.lastExit;
  if (!exit?.reason) return null;
  if (exit.reason.startsWith('signal-')) return null;
  const code = exit.exitCode !== undefined ? ` (exit ${exit.exitCode})` : '';
  const when = exit.at ? ` at ${exit.at}` : '';
  return `Its previous run did not end cleanly: ${exit.reason}${code}${when}.`;
}

/**
 * The cost sentence. A wake starts a meter. The hub CAN stop one now — that is
 * what `nodes.sleep` is, and a wake that never produces a provider gets stopped
 * for you — but the meter still runs from boot until something switches the
 * machine off. Someone tapping this on a phone is spending money; the copy has
 * to say so rather than reading like a refresh icon.
 */
export const WAKE_COST_NOTE =
  'Starts a real machine. It bills from boot until you put it back to sleep.';

/**
 * The sleep sentence, and it names the consequence rather than the saving.
 *
 * Stopping a machine is not the harmless direction of waking one. It ends
 * whatever is running on it — an agent mid-turn, a build, an unflushed
 * transcript — and the shutdown is graceful but it is not a pause. The money is
 * the reason somebody presses it; the work is what they need to be told about.
 */
export const SLEEP_NOTE =
  'Shuts the machine down. Anything still running on it stops, and it stops billing.';

/** Failed wakes, priced honestly. Null when there have been none. */
export function nodeWakeFailureNotice(node: RemoteNodeView): string | null {
  const n = node.wakeFailures ?? 0;
  if (n <= 0) return null;
  // The hub now stops a machine whose wake never produced a provider, so this
  // no longer has to warn about a machine left billing — the hub's own `detail`
  // says whether that stop worked, and it is rendered above this line.
  return `${n} wake${n === 1 ? '' : 's'} failed. The machine started and never became usable — check its boot log.`;
}

// ── the button ──────────────────────────────────────────────────────────────

export interface WakeAffordance {
  /** Render the control at all? False = no control, not a dead one. */
  visible: boolean;
  /** Clickable? A visible-but-disabled control still explains itself. */
  enabled: boolean;
  label: string;
  /** Why it is disabled (or what it will do). Always a full sentence. */
  title: string;
  /** Shown beside a disabled control so the reason survives without a hover —
   *  a tooltip is not an explanation on a phone. */
  reason?: string;
}

/**
 * Should this node get a Connect button, and may this caller press it?
 *
 * The rule that matters: **never show a button that will be refused.**
 * `nodes.wake` is host-authority-only, so a view- or triage-tier phone gets the
 * STATE and not the button — which is the whole silent-failure class this
 * feature exists to remove. Same for `wakeable:false`, which is the hub saying
 * it holds no cloud coordinates or credential for this node: the wake would
 * fail every single time.
 */
export function wakeAffordance(
  node: RemoteNodeView,
  canWake: boolean,
  pending = false,
): WakeAffordance {
  // `stopping` is excluded here for the same reason `waking` is excluded from
  // [[sleepAffordance]], and it was MISSING: the sleep path added the state and
  // this function never learned it, so a draining machine fell through to the
  // final branch and got an ENABLED Connect — rendered beside the disabled
  // "Shutting down…" the sleep affordance was already showing. Two buttons that
  // fight each other on one row, and the enabled one spends money by racing a
  // stop the user just asked for. The transition owns the row until it settles;
  // pinned by contracts/node-view-cases.json's `wakeOffered` column.
  if (node.state === 'available' || node.state === 'stopping') {
    return { visible: false, enabled: false, label: 'Connect', title: '' };
  }
  // The mirror of [[sleepAffordance]]'s own transitional guard, and it comes
  // BEFORE the pending check for the same reason: the hub's state outranks a
  // local optimistic flag. A `stopping` node is draining because somebody just
  // asked it to, and the hub accepts a wake there — which is exactly why the
  // button has to refuse. One click would silently cancel the shutdown and put
  // the meter back on.
  if (node.state === 'stopping') {
    return {
      visible: true,
      enabled: false,
      label: 'Connect',
      title:
        'This machine is shutting down. Starting it now would cancel the stop you just asked for and put it back on the meter, so wait until it is off.',
      reason: 'shutting down',
    };
  }
  if (node.state === 'waking' || pending) {
    return {
      visible: true,
      enabled: false,
      label: 'Starting…',
      title: 'This machine is already starting. Waking it again would do nothing.',
    };
  }
  if (!node.wakeable) {
    return {
      visible: true,
      enabled: false,
      label: 'Connect',
      title: 'This hub holds no cloud credentials for this machine, so it cannot start it.',
      reason: 'no credentials on this hub',
    };
  }
  if (!canWake) {
    return {
      visible: true,
      enabled: false,
      label: 'Connect',
      title: `Starting a machine spends money, so it needs an operator token. ${WAKE_COST_NOTE}`,
      reason: 'needs an operator token',
    };
  }
  return {
    visible: true,
    enabled: true,
    label: 'Connect',
    title: WAKE_COST_NOTE,
  };
}

/**
 * Should this node get a Sleep button, and may this caller press it?
 *
 * The same rule as [[wakeAffordance]] — never show a button that will be
 * refused — applied to the other direction, plus one it does not need:
 *
 *   - only an `available` node gets the control. A node that is already
 *     `stopped` has nothing to stop; one that is `waking` is mid-transition and
 *     a stop pressed there would race the start (the hub does handle it, but a
 *     button that fights the button beside it is not an interface); one that is
 *     `unreachable` is the interesting case and it is handled separately below.
 *   - `unreachable` DOES get it when the hub says the machine may still be
 *     running, because that is the case with a meter attached and it is the
 *     precise reason this button exists.
 *
 * `wakeable` answers both directions — the coordinates and credential a stop
 * needs are the ones a start needs — so it gates this too.
 */
export function sleepAffordance(
  node: RemoteNodeView,
  canSleep: boolean,
  pending = false,
): WakeAffordance {
  // `waking` is excluded EXPLICITLY and not by accident of the running check —
  // a booting machine IS `mayBeRunning` (the hub asked for it to be up), and
  // showing an off switch there would put a button that fights the one beside
  // it on the screen. The transition owns the row until it settles.
  if (node.state === 'waking' || !nodeMayStillBeRunning(node)) {
    return { visible: false, enabled: false, label: 'Put to sleep', title: '' };
  }
  if (node.state === 'stopping' || pending) {
    return {
      visible: true,
      enabled: false,
      label: 'Shutting down…',
      title: 'This machine is already shutting down.',
    };
  }
  if (!node.wakeable) {
    return {
      visible: true,
      enabled: false,
      label: 'Put to sleep',
      title: 'This hub holds no cloud credentials for this machine, so it cannot stop it.',
      reason: 'no credentials on this hub',
    };
  }
  if (!canSleep) {
    return {
      visible: true,
      enabled: false,
      label: 'Put to sleep',
      title: `Stopping a machine ends the work running on it, so it needs an operator token. ${SLEEP_NOTE}`,
      reason: 'needs an operator token',
    };
  }
  return { visible: true, enabled: true, label: 'Put to sleep', title: SLEEP_NOTE };
}

/**
 * Is the hub telling us this machine is probably STILL UP, and therefore still
 * billing?
 *
 * One field, read verbatim. `unreachable` covers both "running and providing
 * nothing" and "off and something is wrong", and offering an off switch for the
 * second is offering a button that does nothing — so the state alone cannot
 * answer this and the hub sends the answer.
 *
 * THE FIRST VERSION OF THIS SNIFFED THE HUB'S `detail` FOR "billing" AND WAS
 * WRONG ON ITS FIRST REAL INPUT: the sentence for a machine the hub had already
 * stopped reads "…so it would not keep billing", which a regex reads as a
 * running machine. A fact the hub holds belongs in a field.
 */
export function nodeMayStillBeRunning(node: RemoteNodeView): boolean {
  return node.state === 'available' || node.mayBeRunning === true;
}

/** A `nodes.sleep` failure, in words a person can act on. Same categories as
 *  [[describeWakeError]] — the hub renders cloud failures by category rather
 *  than by quoting the API, so its own text is safe to show. */
export function describeSleepError(err: unknown): string {
  const msg = (err instanceof Error ? err.message : typeof err === 'string' ? err : '').trim();
  if (isHostAuthorityRefusal(err)) return 'Stopping a machine needs an operator token.';
  if (/unknown node/.test(msg)) return 'This machine is no longer in the registry.';
  if (/cannot be put to sleep from here|has no cloud coordinates or credential/.test(msg))
    return 'This hub holds no cloud credentials for this machine.';
  if (/naming a registered node is required/.test(msg))
    return 'This machine is no longer in the registry.';
  if (isNodeRegistryAbsent(err)) return 'This hub no longer has a node registry.';
  if (!msg) return "Couldn't stop the machine.";
  return msg.replace(/^Error invoking remote method '[^']*': (?:Error: )?/, '');
}

/**
 * A `nodes.wake` failure, in words a person can act on.
 *
 * The hub already renders cloud-API failures by CATEGORY rather than quoting
 * the API's response body, so its text is safe to show; these cases are the
 * ones where the hub's wording describes a client bug and a person needs
 * something better.
 */
export function describeWakeError(err: unknown): string {
  const msg = (err instanceof Error ? err.message : typeof err === 'string' ? err : '').trim();
  if (isHostAuthorityRefusal(err)) return 'Starting a machine needs an operator token.';
  if (/unknown node/.test(msg)) return 'This machine is no longer in the registry.';
  if (/has no cloud coordinates or credential/.test(msg))
    return 'This hub holds no cloud credentials for this machine.';
  if (/naming a registered node is required/.test(msg))
    return 'This machine is no longer in the registry.';
  if (isNodeRegistryAbsent(err)) return 'This hub no longer has a node registry.';
  if (!msg) return "Couldn't start the machine.";
  // Strip the electron ipcMain.handle wrapper so the hub's own sentence shows.
  return msg.replace(/^Error invoking remote method '[^']*': (?:Error: )?/, '');
}

// ── summary ─────────────────────────────────────────────────────────────────

/** Nodes that are NOT quietly fine — the ones worth putting in front of
 *  someone. `available` with a crash notice counts: a node that came back from
 *  a crash is available AND carrying the only notice of that crash. */
export function nodesNeedingAttention(nodes: RemoteNodeView[]): RemoteNodeView[] {
  return nodes.filter((n) => n.state !== 'available' || nodeCrashNotice(n) !== null);
}

/**
 * Which nodes the strip actually renders.
 *
 * [[nodesNeedingAttention]] answers "which of these is not quietly fine", and
 * that stays exactly what it means. This adds the one case the sleep path
 * creates: **a connected machine that this caller can switch off is a machine
 * that is billing right now, and the off switch has to live somewhere.** So it
 * shows when — and only when — there is something the viewer could actually do
 * about it: the hub holds the credential AND this caller holds the authority.
 *
 * A phone on the view or triage tier passes `canSleep:false` and gets byte-for-
 * byte the old behaviour, as does any hub that merely observes a node it cannot
 * power. An ordinary install has no registry at all and renders nothing either
 * way.
 */
export function nodesWorthShowing(nodes: RemoteNodeView[], canSleep: boolean): RemoteNodeView[] {
  return nodes.filter(
    (n) => n.state !== 'available' || nodeCrashNotice(n) !== null || (canSleep && n.wakeable),
  );
}

/** One line for a collapsed strip: "2 machines · 1 asleep". */
export function nodesSummary(nodes: RemoteNodeView[]): string {
  if (!nodes.length) return '';
  const counts = new Map<NodeState, number>();
  for (const n of nodes) counts.set(n.state, (counts.get(n.state) ?? 0) + 1);
  const order: NodeState[] = ['waking', 'stopping', 'unreachable', 'stopped', 'available'];
  const words: Record<NodeState, string> = {
    waking: 'starting',
    stopping: 'shutting down',
    unreachable: 'unreachable',
    stopped: 'asleep',
    available: 'connected',
  };
  const parts = order.filter((s) => counts.get(s)).map((s) => `${counts.get(s)} ${words[s]}`);
  return parts.join(' · ');
}
