import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import policy from '../../../../../contracts/spawn-parameter-support.json';
import keys from '../../../../../contracts/spawn-parameter-keys.json';
import {
  checkDesktopSupport,
  desktopSpawnRoots,
  supportStructure,
  workflowRoots,
} from '../../../tests/support/spawnSupport';
const ROOT = path.resolve(__dirname, '../../../../..');
describe('current spawn support has symmetric source closure', () => {
  it('executes all reviewed rows against actual source and the canonical namespace', () => {
    expect(policy.cases).toHaveLength(51);
    expect(checkDesktopSupport(ROOT, policy)).toEqual([]);
    for (const row of policy.cases) expect(keys.keys).toContain(row.name);
  });
  it('rejects novel and removed fields, blind extraction, and stale inverse exceptions', () => {
    const actual = desktopSpawnRoots(ROOT);
    for (const fields of [[...actual, 'futureField'], actual.filter((f) => f !== 'model'), []])
      expect(checkDesktopSupport(ROOT, policy, fields)).not.toEqual([]);
    const stale = structuredClone(policy);
    Object.assign(stale.differences.desktopOnly, { model: 'old decline' });
    expect(supportStructure(stale)).not.toEqual([]);
    const converged = structuredClone(policy);
    converged.cases.find((r) => r.name === 'remoteCwd')!.rust.observed = true;
    expect(supportStructure(converged)).not.toEqual([]);
  });
  it('rejects contradictory dispositions, missing reasons and obsolete supported intersections', () => {
    const changed = structuredClone(policy);
    changed.cases.find((r) => r.name === 'model')!.rust.disposition = 'refused';
    expect(supportStructure(changed)).not.toEqual([]);
    const reason = structuredClone(policy);
    reason.cases[0].desktop.reason = '';
    expect(supportStructure(reason)).not.toEqual([]);
    const stale = structuredClone(policy);
    stale.mirroredSupported.pop();
    expect(supportStructure(stale)).not.toEqual([]);
    const contradiction = structuredClone(policy);
    contradiction.differences.semantics.targetHub.desktop = 'supported';
    expect(supportStructure(contradiction)).not.toEqual([]);
  });
  it('cannot replace implementation with a comment or a string', () => {
    const file = 'apps/desktop/src/main/services/managedSpawn.ts',
      source = fs.readFileSync(path.join(ROOT, file), 'utf8');
    const changed = source.replace(
      'const wantsFacade = true;',
      `const wantsFacade = false; const decoy="const wantsFacade = true;"; // const wantsFacade = true;`,
    );
    expect(changed).not.toBe(source);
    expect(
      checkDesktopSupport(ROOT, policy, undefined, new Map([[file, changed]])).some((e) =>
        e.includes('support evidence changed mcpFacade'),
      ),
    ).toBe(true);
  });
  it('fails closure when an actual middleware caller field is added', () => {
    const file = 'apps/desktop/src/main/services/fleetWorkflowCore.ts';
    const source = fs.readFileSync(path.join(ROOT, file), 'utf8');
    const changed = source.replace(
      'if (!p.workflowStepId)',
      'p.futureField; if (!p.workflowStepId)',
    );
    expect(changed).not.toBe(source);
    expect(checkDesktopSupport(ROOT, policy, undefined, new Map([[file, changed]]))).toContain(
      'desktop support source closure drift',
    );
  });
  it('discovers new workflow fields, skips shadowed callback bindings, and rejects unknown keys', () => {
    const source =
      'function workflowSpawn(){return async(raw)=>{const p={...(raw as Record<string,unknown>)}; p.futureField; items.some(p=>p.unrelated);}}';
    expect(workflowRoots(source)).toEqual(['futureField']);
    expect(() => workflowRoots(source.replace('p.futureField', 'p[computed()]'))).toThrow(
      'unresolved',
    );
  });
});
