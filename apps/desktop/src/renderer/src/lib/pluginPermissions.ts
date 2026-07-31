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
  'providers.listModels': { label: 'List available models' },
  'providers.checkAll': { label: 'Detect installed agent providers' },
  'replay.open': { label: 'Replay a session timeline' },
  'replay.read': { label: 'Replay a session timeline' },
  'replay.seek': { label: 'Replay a session timeline' },
  'replay.diff': { label: 'Replay a session timeline' },
  'replay.close': { label: 'Replay a session timeline' },
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
  if (!friendly) return { label: path, escapes: false };
  return climbsAboveRoot(segments.slice(1))
    ? { label: `a folder above ${friendly}`, escapes: true }
    : { label: friendly, escapes: false };
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
      lines: consumes.map((c) => ({
        label: c,
        detail: c === '*' ? 'all bus activity' : undefined,
        severity: c === '*' ? 'sensitive' : 'normal',
      })),
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
