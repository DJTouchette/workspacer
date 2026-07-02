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
// Unknown methods fall back to their id and are treated as normal.
const CAP_LABELS: Record<string, { label: string; sensitive?: boolean }> = {
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
};

/** Render a declared path scope in human terms. The `${…}` tokens are the
 *  manifest's dynamic bindings; absolute paths show as-is. */
function scopeLabel(path: string): string {
  if (path.includes('${agentCwd}')) return "the agent's folder";
  if (path.includes('${pluginDir}')) return 'its own folder';
  if (path === '*' || path === '') return 'anywhere';
  return path;
}

function capLine(c: PluginCapability): PermissionLine {
  const method = capabilityMethod(c);
  const known = CAP_LABELS[method];
  const paths = capabilityPaths(c);
  const isFs = method.startsWith('fs.') || method === 'search.project';
  // An fs.* capability with no roots would reach anywhere — flag it (the hub
  // loader rejects this, but disclosure should still call it out if it appears).
  const unscoped = isFs && paths.length === 0;
  return {
    label: known?.label ?? method,
    detail:
      paths.length > 0
        ? `in ${paths.map(scopeLabel).join(', ')}`
        : unscoped
          ? 'anywhere on disk'
          : undefined,
    severity: known?.sensitive || unscoped ? 'sensitive' : 'normal',
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
    groups.push({
      key: 'provide',
      title: 'Answers calls',
      lines: provides.map((p) => ({ label: p, severity: 'normal' as const })),
    });
  }

  return groups;
}

/** True if any declared grant is sensitive — lets a caller show a heads-up
 *  before the itemized list. */
export function hasSensitivePermission(m: PluginManifest): boolean {
  return pluginPermissions(m).some((g) => g.lines.some((l) => l.severity === 'sensitive'));
}
