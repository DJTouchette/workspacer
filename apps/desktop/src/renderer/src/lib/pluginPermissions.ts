// Turn a plugin manifest's declared grants into a grouped, human-readable
// permissions list — the disclosure half of the hub's plugin authorization
// model (the bus enforces exactly these; see services/hub "Plugin
// authorization"). Shown in the install-confirm step and, read-only, in the
// plugins manager, so an install click is informed consent.

import type { PluginManifest, PluginCapability } from '../types/plugin';
import { capabilityMethod, capabilityPaths } from '../types/plugin';

export type PermissionSeverity = 'sensitive' | 'normal';

export interface PermissionLine {
  label: string;
  /** Secondary detail — a filesystem scope, or the raw pattern behind the label. */
  detail?: string;
  severity: PermissionSeverity;
}

export interface PermissionGroup {
  key: 'call' | 'publish' | 'receive' | 'provide';
  title: string;
  lines: PermissionLine[];
}

// Plain-English verb per known capability method, plus whether it's sensitive
// (mutates state, spawns/steers agents, or reaches the filesystem to write).
// An unknown method falls back to its id and is treated as SENSITIVE — see
// capLine. pluginPermissions.test.ts fails if this list falls behind the hub's
// registerCapability registry, which is how terminals.create and claude.approve
// came to display as ordinary.
export const CAP_LABELS: Record<string, { label: string; sensitive?: boolean }> = {
  'fs.read': { label: 'Read files' },
  'fs.listEntries': { label: 'List files' },
  'fs.listDir': { label: 'List files' },
  'fs.watch': { label: 'Watch files for changes' },
  'fs.unwatch': { label: 'Watch files for changes' },
  'search.project': { label: 'Search project files' },
  'fs.write': { label: 'Write & change files', sensitive: true },
  'agents.list': { label: 'See your agents' },
  'agents.sendMessage': { label: 'Send messages to your agents', sensitive: true },
  'agents.spawn': { label: 'Spawn new agents', sensitive: true },
  'agents.kill': { label: 'Terminate agents', sensitive: true },
  'notifications.post': { label: 'Show notifications' },
  'notify.post': { label: 'Show notifications' },

  // Terminals and agent steering — anything here can run commands or change
  // what an agent is about to do.
  'terminals.create': { label: 'Open terminals and run commands', sensitive: true },
  'sessions.terminalInput': { label: "Type into an agent's terminal", sensitive: true },
  'sessions.attachTerminal': { label: "Attach to an agent's terminal", sensitive: true },
  'sessions.detachTerminal': { label: "Detach from an agent's terminal" },
  'sessions.terminalResize': { label: "Resize an agent's terminal" },
  'sessions.terminalKeepalive': { label: "Keep an agent's terminal alive" },
  'claude.approve': { label: 'Approve or deny tool requests on your behalf', sensitive: true },
  'claude.answer': { label: "Answer an agent's questions on your behalf", sensitive: true },
  'claude.gate': { label: 'Gate what an agent may do', sensitive: true },
  'claude.signal': { label: 'Interrupt or stop agents', sensitive: true },
  'claude.setModel': { label: "Change an agent's model", sensitive: true },
  'claude.setEffort': { label: "Change an agent's effort", sensitive: true },
  'claude.setPermissionMode': { label: "Change an agent's permission mode", sensitive: true },
  'claude.handoffBrief': { label: 'Hand an agent off to another provider', sensitive: true },
  'claude.handoffAgentBrief': { label: 'Hand an agent off to another provider', sensitive: true },

  // Reading a session reads its whole conversation.
  'sessions.conversation': { label: 'Read your agent conversations' },
  'sessions.transcript': { label: 'Read your agent transcripts' },
  'sessions.snapshot': { label: 'Read agent state' },
  'sessions.snapshots': { label: 'Read agent state' },
  'sessions.recent': { label: 'See your recent sessions' },

  // Git — the write side changes the work tree and can publish it.
  'git.status': { label: 'See git status' },
  'git.log': { label: 'Read git history' },
  'git.diff': { label: 'Read git diffs' },
  'git.numstat': { label: 'Read git diffs' },
  'git.commitDiff': { label: 'Read git diffs' },
  'git.commitNumstat': { label: 'Read git diffs' },
  'git.stage': { label: 'Stage changes', sensitive: true },
  'git.unstage': { label: 'Unstage changes', sensitive: true },
  'git.commit': { label: 'Commit changes', sensitive: true },
  'git.push': { label: 'Push commits to the remote', sensitive: true },

  // Read-only discovery and history.
  'fs.readImage': { label: 'Read image files' },
  'app.getCwd': { label: 'See the app working directory' },
  'app.supervisorHome': { label: 'See the supervisor folder' },
  'analytics.recent': { label: 'Read usage history' },
  'analytics.summary': { label: 'Read usage history' },
  // NOT read-only, whatever the label says: this RUNS the provider CLI in a
  // directory the caller names, and opencode loads and executes every
  // `<cwd>/.opencode/plugin/*.js` at startup before printing anything. The cwd
  // is confined to the browse roots now (capspec.PathParam), but "a program runs"
  // is a consent question on its own — a `sensitive: false` on a method that
  // executes something is the one way this list can under-warn, since capLine()
  // only fails closed for methods it has never heard of.
  'providers.listModels': { label: 'Run a provider CLI to list its models', sensitive: true },
  'providers.checkAll': { label: 'Detect installed agent providers' },
  'replay.open': { label: 'Replay a session timeline' },
  'replay.read': { label: 'Replay a session timeline' },
  'replay.seek': { label: 'Replay a session timeline' },
  'replay.diff': { label: 'Replay a session timeline' },
  'replay.close': { label: 'Replay a session timeline' },

  // The HUB-NATIVE capabilities. There are THREE capability registries, not two:
  // the desktop's hubCapabilities.ts, the brain's handlers.go, and cmd/hub's own
  // RegisterLocal/RegisterLocalIdent calls. These seven belong to the third, so
  // they appeared in NEITHER provider's method list — which is why the drift
  // guard below (which reads hubCapabilities.ts) could never have named them,
  // and why they had no row here at all. `layout.set` is the one that matters:
  // the shared document it writes is respawned verbatim by the desktop on its
  // next launch, so its per-agent fields are arguments to a LOCAL spawn.
  'layout.get': { label: 'Read the shared workspace layout' },
  'layout.set': { label: 'Change the shared workspace layout', sensitive: true },
  'push.key': { label: 'Read the push notification public key' },
  'push.subscribe': { label: 'Send push notifications to a device', sensitive: true },
  'push.unsubscribe': { label: 'Stop push notifications to a device' },
  'push.list': { label: 'See subscribed push devices' },
  'push.revoke': { label: 'Remove a push subscription', sensitive: true },
  // A send TRIGGER, but nothing about the message is caller-supplied — title and
  // body are literals in the hub's RPCTest — so it puts no attacker-chosen text
  // on a lock screen. Not marked sensitive for that reason: the tier that may
  // subscribe can already provoke real pushes by approving or answering.
  'push.test': { label: 'Send a test push notification' },

  // The CATALOG capabilities. hubCapabilities.ts registers through TWO helpers —
  // registerCapability() and the delegation-aware alias cat() — and the drift
  // guard below only ever matched the first, so these twenty registered methods
  // were outside the list that claims to cover "every capability the main process
  // actually registers". capspec's own parser for the same file has always known
  // both spellings, which is what makes the omission an oversight rather than a
  // decision. config.save in particular is the method whose own capspec entry
  // says agents.binaries is "argv[0] of every spawned agent".
  'plugins.tools': { label: 'List the agent tools installed plugins contribute' },
  'config.get': { label: 'Read your app settings' },
  'config.getPath': { label: 'See where your settings file lives' },
  'config.reload': { label: 'Reload your app settings' },
  'config.save': { label: 'Change your app settings', sensitive: true },
  'claude.listModels': { label: 'List available models' },
  'claude.profiles.list': { label: 'See your Claude profiles' },
  'claude.profiles.add': { label: 'Add a Claude profile', sensitive: true },
  'claude.profiles.update': { label: 'Change a Claude profile', sensitive: true },
  'claude.profiles.remove': { label: 'Remove a Claude profile', sensitive: true },
  'claude.sessionsForDir': { label: 'See past sessions for a folder' },
  'sessions.list': { label: 'See your saved sessions' },
  'sessions.load': { label: 'Read a saved session' },
  'sessions.save': { label: 'Save a session', sensitive: true },
  'sessions.delete': { label: 'Delete a saved session', sensitive: true },
  'layouts.list': { label: 'See your saved layouts' },
  'layouts.save': { label: 'Save a layout', sensitive: true },
  'layouts.delete': { label: 'Delete a saved layout', sensitive: true },
  'library.list': { label: 'Read your prompt library' },
  'library.save': { label: 'Write to your prompt library', sensitive: true },
  'library.remove': { label: 'Delete from your prompt library', sensitive: true },
};

/** Plain-English name for the folder a `${…}` binding resolves to. */
const BINDING_LABELS: Record<string, string> = {
  '${agentCwd}': "the agent's folder",
  '${pluginDir}': 'its own folder',
};

/** Does the path climb above its own first segment? Tracks depth below the
 *  root: the scope is out of bounds the moment a `..` would take it negative. */
function climbsAboveRoot(segments: string[]): boolean {
  let depth = 0;
  for (const seg of segments) {
    if (seg === '..') {
      if (depth === 0) return true;
      depth -= 1;
    } else if (seg !== '.' && seg !== '') {
      depth += 1;
    }
  }
  return false;
}

interface ScopeInfo {
  label: string;
  /** The scope reaches outside the folder its binding names. */
  escapes: boolean;
}

/**
 * Describe a declared path scope in human terms. The `${…}` tokens are the
 * manifest's dynamic bindings; absolute paths show as-is.
 *
 * The label describes where the scope RESOLVES to, not which token it is
 * spelled with. A manifest is free to write `${pluginDir}/../..` — the config
 * directory, which holds the remote token — and matching on the token text
 * alone showed that to the user as "its own folder", the reassuring opposite of
 * what was being declared. (The hub refuses to expand a scope containing `..`,
 * so such a capability is granted no root at all; that's enforcement, and it is
 * not a reason for the consent dialog to describe the declaration wrongly.)
 */
function describeScope(path: string): ScopeInfo {
  if (path === '*' || path === '') return { label: 'anywhere', escapes: true };
  const segments = path.replace(/\\/g, '/').split('/');
  const friendly = BINDING_LABELS[segments[0]];
  if (friendly) {
    return climbsAboveRoot(segments.slice(1))
      ? { label: `a folder above ${friendly}`, escapes: true }
      : { label: friendly, escapes: false };
  }
  // An ABSOLUTE scope, which is the case this function used to get exactly
  // backwards. The two spellings it flagged — `*` and `${…}/../..` — are the two
  // the hub resolves to NO root at all (expandScope returns ""), while `/` was
  // rendered "in /" at severity normal even though the bus stores it verbatim as
  // a grant root and a volume root contains everything below it. So the scope
  // that granted the whole filesystem read as an ordinary line while the two
  // that granted nothing read as sensitive, in a disclosure whose own contract
  // is that under-warning is the failure mode that matters.
  //
  // An absolute scope is outside every folder the plugin's own bindings name, by
  // construction — it is not `${pluginDir}` and not `${agentCwd}` — so it is a
  // filesystem grant the user is being asked to hand over on install-time
  // consent alone. It says so.
  const volumeRoot = /^(?:\/|[A-Za-z]:\/?)$/.test(path.replace(/\\/g, '/'));
  if (volumeRoot) return { label: 'the WHOLE filesystem', escapes: true };
  return { label: `${path} (outside its own folder)`, escapes: true };
}

export function capLine(c: PluginCapability): PermissionLine {
  const method = capabilityMethod(c);
  const known = CAP_LABELS[method];
  const paths = capabilityPaths(c);
  const isFs = method.startsWith('fs.') || method === 'search.project';
  // An fs.* capability with no roots would reach anywhere — flag it (the hub
  // loader rejects this, but disclosure should still call it out if it appears).
  const unscoped = isFs && paths.length === 0;
  const scopes = paths.map(describeScope);
  return {
    label: known?.label ?? method,
    detail:
      scopes.length > 0
        ? `in ${scopes.map((s) => s.label).join(', ')}`
        : unscoped
          ? 'anywhere on disk'
          : undefined,
    severity:
      // Unlabelled methods read as SENSITIVE, not normal. CAP_LABELS is a
      // hand-maintained subset of the hub's registry (registerCapability in
      // hubCapabilities.ts), and it fell far behind — terminals.create,
      // sessions.terminalInput and claude.approve all displayed as ordinary
      // because nobody had added a row for them. Failing closed means the worst
      // a stale list can do is over-warn about something harmless, instead of
      // under-warning about the one that runs commands.
      known === undefined || known.sensitive || unscoped || scopes.some((s) => s.escapes)
        ? 'sensitive'
        : 'normal',
  };
}

/** A `command.*` or blanket `*` emit lets a plugin drive the app; a blanket `*`
 *  consume means it sees all bus traffic. Those are the event patterns worth
 *  flagging. */
function isBroad(pattern: string): boolean {
  return pattern === '*' || pattern.startsWith('command.');
}

/** How the hub answers "may a non-trusted credential receive this topic" —
 *  capspec.EventTopicDisposition, spelled the same on both sides of the wire. */
export type TopicDisposition = 'guarded-by-capability' | 'host-only' | 'open-by-decision';

export interface EventTopicRule {
  pattern: string;
  disposition: TopicDisposition;
  /** Set only on `guarded-by-capability` rows: the capability that owns the payload. */
  method?: string;
}

/**
 * THE EVENT-TOPIC REGISTRY, renderer copy.
 *
 * The enforcing copy is services/hub/internal/capspec/eventtopics.go (mayConsume
 * and mayPublish read it); this is the DISCLOSING copy, and
 * `contracts/event-topic-consent-cases.json` pins the two together — each side
 * has a test that fails if it drifts from the fixture, so neither can move
 * alone. The `reason` field of each row is deliberately NOT duplicated here: the
 * argument for a classification belongs with the code that enforces it.
 *
 * WHAT THIS REPLACES, and why the shape changed. This was a hand-copied table of
 * the five `guarded-by-capability` rows and nothing else. Every `host-only` row
 * was missing, so `consumes: ["plugin.log", "sidecar.*"]` rendered as two
 * ORDINARY lines with hasSensitivePermission() === false — plugin.log being one
 * verbatim, unredacted line of a sidecar's stdout/stderr (whose environment
 * carries WKS_SETTINGS with secret plugin settings in plaintext) and
 * plugin.loaded the whole Manifest: install argv, the sidecar's server command,
 * and every declared filesystem scope, i.e. a map of what each sidecar may
 * reach. A partial copy of a security table is worse than no copy, because it
 * looks like an answer.
 *
 * The consent dialog was also the stated justification for plugins being exempt
 * from the registry at all — "a plugin's event reach is its manifest's
 * `consumes`, declared at install and shown in the consent dialog, which is a
 * real answer to the same question". The exemption is gone (the bus consults the
 * registry on the plugin arm too), but a dialog that describes a terminal-stream
 * subscription, or a request for the host's sidecar logs, as ordinary is still
 * lying about what is being asked for.
 */
export const EVENT_TOPIC_RULES: EventTopicRule[] = [
  { pattern: 'agent.snapshot', disposition: 'open-by-decision' },
  { pattern: 'agent.state_changed', disposition: 'open-by-decision' },
  // command.* — UI-navigation requests (focus an agent, open a pane/plugin,
  // pre-fill the spawn dialog). Open by decision: a command event carries only
  // its own arguments, never host state; publish-direction abuse is bounded by
  // the manifest emits list like any other topic.
  { pattern: 'command.focus_agent', disposition: 'open-by-decision' },
  { pattern: 'command.open_pane', disposition: 'open-by-decision' },
  { pattern: 'command.open_plugin', disposition: 'open-by-decision' },
  { pattern: 'command.open_spawn_dialog', disposition: 'open-by-decision' },
  {
    pattern: 'agent.statusline',
    disposition: 'guarded-by-capability',
    method: 'sessions.snapshot',
  },
  { pattern: 'fs.changed', disposition: 'guarded-by-capability', method: 'fs.watch' },
  { pattern: 'layout.changed', disposition: 'open-by-decision' },
  { pattern: 'library.changed', disposition: 'open-by-decision' },
  { pattern: 'plugin.install.progress', disposition: 'host-only' },
  { pattern: 'plugin.loaded', disposition: 'host-only' },
  { pattern: 'plugin.log', disposition: 'host-only' },
  { pattern: 'plugin.sandbox.refused', disposition: 'host-only' },
  { pattern: 'plugin.sandboxed', disposition: 'host-only' },
  { pattern: 'plugin.settings.changed', disposition: 'host-only' },
  { pattern: 'plugin.unloaded', disposition: 'host-only' },
  { pattern: 'plugin.unsandboxed', disposition: 'host-only' },
  {
    pattern: 'pty.bytes.*',
    disposition: 'guarded-by-capability',
    method: 'sessions.attachTerminal',
  },
  {
    pattern: 'pty.desync',
    disposition: 'guarded-by-capability',
    method: 'sessions.attachTerminal',
  },
  { pattern: 'pty.exit', disposition: 'guarded-by-capability', method: 'sessions.attachTerminal' },
  { pattern: 'sidecar.*', disposition: 'host-only' },
  { pattern: 'workflow.agent.finished', disposition: 'open-by-decision' },
  { pattern: 'workflow.completed', disposition: 'open-by-decision' },
  { pattern: 'workflow.failed', disposition: 'open-by-decision' },
  { pattern: 'workflow.started', disposition: 'open-by-decision' },
];

/** Does a manifest's consume PATTERN reach a registry ROW? The bus's own rule —
 *  exact, or a trailing '*' over a prefix — applied in both directions, since a
 *  manifest may declare `pty.*` (or `plugin.*`) and mean the whole family. */
function reaches(pattern: string, rule: EventTopicRule): boolean {
  if (rule.pattern === pattern) return true;
  const rulePrefix = rule.pattern.endsWith('*') ? rule.pattern.slice(0, -1) : undefined;
  const askPrefix = pattern.endsWith('*') ? pattern.slice(0, -1) : undefined;
  if (rulePrefix !== undefined && pattern.startsWith(rulePrefix)) return true;
  if (askPrefix !== undefined && rule.pattern.startsWith(askPrefix)) return true;
  return false;
}

/**
 * The registry row a consume pattern reaches, if any.
 *
 * A wildcard can span several rows (`agent.*` covers the guarded
 * agent.statusline AND the open agent.snapshot), and disclosure takes the LOUDER
 * one: an exact row first, then a capability-guarded one, then host-only, and
 * only an all-open span reads as ordinary. Picking the first match in table
 * order would make a security label depend on where somebody inserted a row.
 */
export function topicRuleFor(pattern: string): EventTopicRule | undefined {
  const hits = EVENT_TOPIC_RULES.filter((r) => reaches(pattern, r));
  return (
    hits.find((r) => r.pattern === pattern) ??
    hits.find((r) => r.disposition === 'guarded-by-capability') ??
    hits.find((r) => r.disposition === 'host-only') ??
    hits[0]
  );
}

/** The capability a consume pattern's topic requires, if any. Only
 *  `guarded-by-capability` rows have one — a host-only topic is refused outright
 *  and no capability unlocks it, so a caller asking this question must not read
 *  `undefined` as "ordinary". */
export function capabilityBehindTopic(pattern: string): string | undefined {
  for (const rule of EVENT_TOPIC_RULES) {
    if (rule.disposition !== 'guarded-by-capability') continue;
    if (reaches(pattern, rule)) return rule.method;
  }
  return undefined;
}

export function pluginPermissions(m: PluginManifest): PermissionGroup[] {
  const groups: PermissionGroup[] = [];

  const caps = m.capabilities ?? [];
  if (caps.length > 0) {
    groups.push({
      key: 'call',
      title: 'Can',
      lines: caps.map(capLine),
    });
  }

  const emits = m.emits ?? [];
  if (emits.length > 0) {
    groups.push({
      key: 'publish',
      title: 'Publishes events',
      lines: emits.map((e) => ({
        label: e,
        detail: isBroad(e) ? 'can drive the app' : undefined,
        severity: isBroad(e) ? 'sensitive' : 'normal',
      })),
    });
  }

  const consumes = m.consumes ?? [];
  if (consumes.length > 0) {
    groups.push({
      key: 'receive',
      title: 'Receives events',
      lines: consumes.map((c) => {
        if (c === '*') return { label: c, detail: 'all bus activity', severity: 'sensitive' };
        const rule = topicRuleFor(c);
        if (rule?.disposition === 'guarded-by-capability' && rule.method)
          return {
            label: c,
            detail: `the output of ${CAP_LABELS[rule.method]?.label ?? rule.method} — requires that capability`,
            severity: 'sensitive',
          };
        // HOST-ONLY. No capability returns these payloads, so there is no
        // capability to name and the old table had no row for them at all —
        // which is how a request for a sidecar's unredacted stderr, or for the
        // Manifest listing every filesystem scope each sidecar holds, rendered
        // as an ordinary line. The hub refuses the topic to a plugin, and that
        // is not a reason to describe the ASK as ordinary: a manifest that wants
        // the host's process/confinement plane is telling the user something
        // about itself.
        if (rule?.disposition === 'host-only')
          return {
            label: c,
            detail: 'host-internal state — only the app itself receives this',
            severity: 'sensitive',
          };
        return { label: c, detail: undefined, severity: 'normal' };
      }),
    });
  }

  const provides = m.provides ?? [];
  if (provides.length > 0) {
    // Every provides entry is sensitive. Registering as a provider puts the
    // plugin in the answer path for a capability the app and other plugins
    // call, and the caller acts on what comes back — which is more than an
    // emits/consumes pattern (already flagged when broad) buys. Rendering
    // "Answers calls: *" at 'normal' meant the broadest grant in the manifest
    // was the one line with no warning on it, and hasSensitivePermission()
    // stayed false for a plugin that answers everything.
    groups.push({
      key: 'provide',
      title: 'Answers calls',
      lines: provides.map((p) => ({
        label: p,
        detail: p.includes('*') ? 'stands in for any matching capability' : undefined,
        severity: 'sensitive' as const,
      })),
    });
  }

  return groups;
}

/** True if any declared grant is sensitive — lets a caller show a heads-up
 *  before the itemized list. */
export function hasSensitivePermission(m: PluginManifest): boolean {
  return pluginPermissions(m).some((g) => g.lines.some((l) => l.severity === 'sensitive'));
}
