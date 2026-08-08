import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import * as path from 'path';
import {
  pluginPermissions,
  hasSensitivePermission,
  CAP_LABELS,
  capLine,
} from '../src/lib/pluginPermissions';
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
    const [sub] = pluginPermissions(
      mf({ capabilities: [{ method: 'fs.read', paths: ['${pluginDir}/data'] }] }),
    );
    expect(sub.lines[0]).toMatchObject({ detail: 'in its own folder', severity: 'normal' });
  });

  // The dialog is consent: it has to describe where the scope RESOLVES, not
  // which token it is spelled with. `${pluginDir}/../..` is the config
  // directory (remote-token lives there) and used to read as "its own folder".
  it('describes a scope that climbs out of its binding as outside it, and flags it', () => {
    const [call] = pluginPermissions(
      mf({ capabilities: [{ method: 'fs.read', paths: ['${pluginDir}/../..'] }] }),
    );
    expect(call.lines[0]).toMatchObject({
      detail: 'in a folder above its own folder',
      severity: 'sensitive',
    });
    expect(
      hasSensitivePermission(
        mf({ capabilities: [{ method: 'fs.read', paths: ['${pluginDir}/../..'] }] }),
      ),
    ).toBe(true);
  });

  it('does not flag `..` that stays inside the binding', () => {
    const [call] = pluginPermissions(
      mf({ capabilities: [{ method: 'fs.read', paths: ['${pluginDir}/data/../cache'] }] }),
    );
    expect(call.lines[0]).toMatchObject({ detail: 'in its own folder', severity: 'normal' });
  });

  it('treats a wildcard scope as reaching anywhere', () => {
    const [call] = pluginPermissions(mf({ capabilities: [{ method: 'fs.read', paths: ['*'] }] }));
    expect(call.lines[0]).toMatchObject({ detail: 'in anywhere', severity: 'sensitive' });
  });

  // The disclosure used to be INVERTED relative to what the hub enforces: the two
  // spellings it flagged sensitive (`*` and `${…}/../..`) are the two expandScope
  // resolves to NO root at all, while `/` — which the bus stores verbatim as a
  // grant root, and a volume root contains everything below it — rendered "in /"
  // at severity normal. The old test asserted only `detail`, never `severity`, so
  // it read as coverage.
  it('shows an absolute scope as written, and says it is outside the plugin', () => {
    const [call] = pluginPermissions(
      mf({ capabilities: [{ method: 'fs.read', paths: ['/etc/hosts'] }] }),
    );
    expect(call.lines[0].detail).toContain('/etc/hosts');
    expect(call.lines[0].severity).toBe('sensitive');
  });

  it('a scope of / is disclosed as the whole filesystem, not as an ordinary line', () => {
    for (const root of ['/', 'C:\\', 'C:/']) {
      const [call] = pluginPermissions(
        mf({ capabilities: [{ method: 'fs.read', paths: [root] }] }),
      );
      expect(call.lines[0].detail, `scope ${root}`).toBe('in the WHOLE filesystem');
      expect(call.lines[0].severity, `scope ${root}`).toBe('sensitive');
    }
    // …and it badges the plugin in the gallery and the install-confirm step.
    expect(
      hasSensitivePermission(mf({ capabilities: [{ method: 'fs.read', paths: ['/'] }] })),
      'a whole-filesystem grant installed with no sensitivity badge',
    ).toBe(true);
  });

  it('still describes a plugin-owned scope as ordinary', () => {
    // The floor: making every scope sensitive would be the same defect pointed
    // the other way — a dialog that shouts at everything discloses nothing.
    const [call] = pluginPermissions(
      mf({ capabilities: [{ method: 'fs.read', paths: ['${pluginDir}/data'] }] }),
    );
    expect(call.lines[0]).toMatchObject({ detail: 'in its own folder', severity: 'normal' });
  });

  it('flags an unscoped fs.* capability as reaching anywhere on disk', () => {
    const [call] = pluginPermissions(mf({ capabilities: ['fs.write'] }));
    expect(call.lines[0]).toMatchObject({ severity: 'sensitive', detail: 'anywhere on disk' });
  });

  it('falls back to the raw method id for unknown capabilities, and warns', () => {
    // This used to assert severity 'normal'. That fail-open default is what let
    // terminals.create and claude.approve — both absent from CAP_LABELS at the
    // time — render as ordinary lines in the install dialog. An unlabelled
    // capability is now treated as sensitive, so a stale list over-warns
    // instead of under-warning.
    const [call] = pluginPermissions(mf({ capabilities: ['custom.thing'] }));
    expect(call.lines[0]).toMatchObject({ label: 'custom.thing', severity: 'sensitive' });
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

  // Answering a capability call puts the plugin in the path of something the
  // app or another plugin will act on — strictly more than publishing an event.
  // Every provides line is disclosed as sensitive; `*` (answer everything) was
  // the broadest grant a manifest can ask for and rendered with no warning.
  it('flags every provides entry as sensitive, wildcards louder', () => {
    const [prov] = pluginPermissions(mf({ provides: ['recon.overview', 'recon.*', '*'] }));
    expect(prov.lines.map((l) => l.severity)).toEqual(['sensitive', 'sensitive', 'sensitive']);
    expect(prov.lines[0].detail).toBeUndefined();
    expect(prov.lines[2].detail).toBe('stands in for any matching capability');
  });

  it('hasSensitivePermission reflects any sensitive line', () => {
    expect(hasSensitivePermission(mf({ capabilities: ['agents.list'] }))).toBe(false);
    expect(hasSensitivePermission(mf({ capabilities: ['agents.spawn'] }))).toBe(true);
    expect(hasSensitivePermission(mf({ emits: ['command.*'] }))).toBe(true);
    expect(hasSensitivePermission(mf({ provides: ['*'] }))).toBe(true);
    expect(hasSensitivePermission(mf({}))).toBe(false);
  });
});

describe('CAP_LABELS drift guard', () => {
  // The label map is a hand-maintained mirror of the hub's capability registry.
  // It fell 38 methods behind, so terminals.create, sessions.terminalInput and
  // claude.approve — each of which runs commands or acts for the user —
  // displayed in the install dialog as ordinary, unhighlighted lines.
  it('labels every capability the main process actually registers', () => {
    const source = readFileSync(
      path.join(__dirname, '../../main/services/hubCapabilities.ts'),
      'utf-8',
    );
    // BOTH registration helpers. hubCapabilities.ts registers through
    // registerCapability() and through the delegation-aware alias cat(), and this
    // regex matched only the first — so a third of the surface (config.save,
    // library.save, sessions.delete, layouts.delete and seventeen more) was
    // outside a guard whose name says "every capability the main process actually
    // registers", and the `> 20` canary passed comfortably at 49 while saying
    // nothing about the 24 it could not see. capspec's parser for the same file
    // has always matched `(?:registerCapability|cat)\(`.
    const registered = [
      ...source.matchAll(/(?:registerCapability|cat)\(\s*'([a-zA-Z][\w.]*)'/g),
    ].map((m) => m[1]);
    expect(
      registered.length,
      'the registry regex found nothing — has it been renamed?',
    ).toBeGreaterThan(60);
    expect(
      registered.filter((m) => m.startsWith('config.')).length,
      'the cat()-registered catalog capabilities are invisible to this guard again',
    ).toBeGreaterThan(0);

    const missing = registered.filter((m) => !(m in CAP_LABELS));
    expect(
      missing,
      'these capabilities have no plain-English label, so consent shows a raw method id',
    ).toEqual([]);
  });

  // THE THIRD REGISTRY. The guard above reads hubCapabilities.ts, and the Go
  // half reads cmd/brain's method lists; between them they cover the 73 methods
  // a PROVIDER answers. cmd/hub registers seven of its own with
  // RegisterLocal/RegisterLocalIdent, and they are in neither provider's list —
  // so no guard on either side of the language boundary had ever enumerated
  // them, and none of the seven had a row here. `layout.set` is what that cost:
  // the shared document it writes is adopted and respawned verbatim by the
  // desktop on its next launch, so its per-agent fields are arguments to a LOCAL
  // spawn, and a plugin manifest declaring the method displayed no line at all.
  //
  // TWIN: capspec's TestHubNativeCapabilitiesAllClassified, which holds the same
  // registrations to the machine-enforced record.
  it('labels every capability the hub itself registers', () => {
    const source = readFileSync(
      path.join(__dirname, '../../../../../services/hub/cmd/hub/main.go'),
      'utf-8',
    );
    // BOTH doors: layout.set moved from RegisterLocal to RegisterLocalIdent the
    // day it learned to scrub a non-trusted write, so a regex matching only the
    // first would have lost the method this test exists for — the same defect the
    // guard above had with cat().
    const registered = [
      ...source.matchAll(/RegisterLocal(?:Ident)?\(\s*'?"([a-zA-Z][\w.]*)"/g),
    ].map((m) => m[1]);
    // A RATCHET, not a canary: a scan that finds nothing asserts nothing, and
    // "nothing looked at this file" is exactly the state layout.set shipped in.
    expect(
      new Set(registered).size,
      'the hub-native registry regex found fewer methods than the hub registers — has RegisterLocal been renamed or the registrations moved?',
    ).toBeGreaterThanOrEqual(7);
    for (const must of ['layout.get', 'layout.set']) {
      expect(
        registered,
        'the floor is met by other methods while the one this guard exists for went unscanned',
      ).toContain(must);
    }
    const missing = registered.filter((m) => !(m in CAP_LABELS));
    expect(
      missing,
      'these hub-native capabilities have no plain-English label, so consent shows a raw method id',
    ).toEqual([]);
  });

  it('treats an unlabelled capability as sensitive, not normal', () => {
    // Failing closed is what makes the guard above a warning rather than a hole:
    // the worst a stale list can do is over-warn.
    const line = capLine({ method: 'some.brand.new.capability' } as never);
    expect(line.severity).toBe('sensitive');
  });
});
