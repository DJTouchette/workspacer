import { describe, it, expect } from 'vitest';
import { pluginPermissions, hasSensitivePermission } from '../src/lib/pluginPermissions';
import type { PluginManifest } from '../src/types/plugin';

function mf(partial: Partial<PluginManifest>): PluginManifest {
  return { id: 'p', name: 'P', apiVersion: '1', ...partial };
}

describe('pluginPermissions', () => {
  it('groups the four grant kinds and omits empty ones', () => {
    const groups = pluginPermissions(
      mf({
        capabilities: ['agents.list'],
        emits: ['rules.fired'],
        consumes: ['agent.state_changed'],
        provides: ['recon.overview'],
      }),
    );
    expect(groups.map((g) => g.key)).toEqual(['call', 'publish', 'receive', 'provide']);

    const noEvents = pluginPermissions(mf({ capabilities: ['agents.list'] }));
    expect(noEvents.map((g) => g.key)).toEqual(['call']);
  });

  it('labels known capabilities in plain English', () => {
    const [call] = pluginPermissions(mf({ capabilities: ['agents.list', 'notifications.post'] }));
    expect(call.lines.map((l) => l.label)).toEqual(['See your agents', 'Show notifications']);
  });

  it('flags write/spawn/steer capabilities as sensitive, reads as normal', () => {
    const [call] = pluginPermissions(
      mf({
        capabilities: [
          { method: 'fs.read', paths: ['${agentCwd}'] },
          { method: 'fs.write', paths: ['${agentCwd}'] },
          'agents.spawn',
        ],
      }),
    );
    const bySeverity = Object.fromEntries(call.lines.map((l) => [l.label, l.severity]));
    expect(bySeverity['Read files']).toBe('normal');
    expect(bySeverity['Write & change files']).toBe('sensitive');
    expect(bySeverity['Spawn new agents']).toBe('sensitive');
  });

  it('renders path scopes with friendly binding names', () => {
    const [call] = pluginPermissions(
      mf({ capabilities: [{ method: 'fs.write', paths: ['${agentCwd}', '${pluginDir}'] }] }),
    );
    expect(call.lines[0].detail).toBe("in the agent's folder, its own folder");
  });

  it('flags an unscoped fs.* capability as reaching anywhere on disk', () => {
    const [call] = pluginPermissions(mf({ capabilities: ['fs.write'] }));
    expect(call.lines[0]).toMatchObject({ severity: 'sensitive', detail: 'anywhere on disk' });
  });

  it('falls back to the raw method id for unknown capabilities', () => {
    const [call] = pluginPermissions(mf({ capabilities: ['custom.thing'] }));
    expect(call.lines[0]).toMatchObject({ label: 'custom.thing', severity: 'normal' });
  });

  it('flags command.* / * emits as app-driving, others normal', () => {
    const [pub] = pluginPermissions(mf({ emits: ['command.*', 'rules.fired'] }));
    const bySeverity = Object.fromEntries(pub.lines.map((l) => [l.label, l.severity]));
    expect(bySeverity['command.*']).toBe('sensitive');
    expect(bySeverity['rules.fired']).toBe('normal');
    expect(pub.lines.find((l) => l.label === 'command.*')?.detail).toBe('can drive the app');
  });

  it('flags a blanket * consume as seeing all bus activity', () => {
    const [recv] = pluginPermissions(mf({ consumes: ['*'] }));
    expect(recv.lines[0]).toMatchObject({ severity: 'sensitive', detail: 'all bus activity' });
    const [scoped] = pluginPermissions(mf({ consumes: ['agent.*'] }));
    expect(scoped.lines[0].severity).toBe('normal');
  });

  it('hasSensitivePermission reflects any sensitive line', () => {
    expect(hasSensitivePermission(mf({ capabilities: ['agents.list'] }))).toBe(false);
    expect(hasSensitivePermission(mf({ capabilities: ['agents.spawn'] }))).toBe(true);
    expect(hasSensitivePermission(mf({ emits: ['command.*'] }))).toBe(true);
    expect(hasSensitivePermission(mf({}))).toBe(false);
  });
});
