import { describe, expect, it } from 'vitest';
import path from 'node:path';
import ts from 'typescript';
import keys from '../../../../../contracts/spawn-parameter-keys.json';
import historical from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import { desktopRegistrations } from '../../../tests/support/capabilitySource';
import { desktopParameterFields } from '../../../tests/support/parameterBindings';
const ROOT = path.resolve(__dirname, '../../../../..');
function closure(fields: string[], registry: string[] = keys.keys): string[] {
  const errors: string[] = [];
  if (fields.length < 20) errors.push('desktop spawn source population collapsed');
  for (const field of fields)
    if (!registry.includes(field)) errors.push('missing canonical spawn key: ' + field);
  return errors;
}
function actual(): string[] {
  const registrations = desktopRegistrations(ROOT).filter((row) => row.method === 'agents.spawn');
  if (registrations.length !== 1) throw Error('expected one desktop spawn provider');
  const row = registrations[0];
  return desktopParameterFields(row.node.arguments[1], row.source, true);
}
function synthetic(body: string): string[] {
  const source = ts.createSourceFile(
    'fixture.ts',
    `const handler=(params:unknown)=>{${body}}`,
    ts.ScriptTarget.Latest,
    true,
  );
  return desktopParameterFields(
    (source.statements[0] as ts.VariableStatement).declarationList.declarations[0].initializer!,
    source,
    true,
  );
}
describe('canonical spawn spelling closes over actual provider fields', () => {
  it('retains historical keys and reviewed reservations and covers the desktop AST surface', () => {
    expect(keys.keys).toHaveLength(52);
    expect(new Set(keys.keys).size).toBe(52);
    expect(historical.spawnKeys).toHaveLength(46);
    for (const key of historical.spawnKeys) expect(keys.keys).toContain(key);
    expect(keys.keys.filter((key) => !historical.spawnKeys.includes(key)).sort()).toEqual(
      Object.keys(keys.reservations).sort(),
    );
    expect(Object.values(keys.reservations).every((reason) => reason.trim().length > 0)).toBe(true);
    const executed = new Set<string>();
    for (const row of keys.cases) {
      expect(keys.keys).toContain(row.key);
      expect(row.alias).toBe(row.key.toUpperCase());
      expect(keys.keys).not.toContain(row.alias);
      expect(row.why.trim().length).toBeGreaterThan(0);
      executed.add(row.key);
    }
    expect(executed.size).toBe(6);
    expect([...executed].sort()).toEqual(Object.keys(keys.reservations).sort());
    expect(closure(actual())).toEqual([]);
  });
  it('keeps wire rename keys and root provenance without mistaking nested keys for spawn parameters', () => {
    expect(
      synthetic(
        'const { provider:reqProvider, resultSchema:{properties} } = params as { provider:string; resultSchema:{properties:unknown} }; const { value }=(params as any).templateParams;',
      ),
    ).toEqual(['provider', 'resultSchema', 'templateParams']);
  });
  it('rejects a removed canonical key, added source field, and blind source parser', () => {
    const fields = actual();
    expect(
      closure(
        fields,
        keys.keys.filter((key) => key !== 'modelIdentity'),
      ),
    ).toContain('missing canonical spawn key: modelIdentity');
    const declarations = keys.keys.map((key) => `const ${key}=(params as any).${key};`).join('\n');
    const changed = synthetic(declarations + '\nconst {newProviderFlag: renamed}=params as any;');
    expect(closure(changed)).toContain('missing canonical spawn key: newProviderFlag');
    expect(closure([])).toContain('desktop spawn source population collapsed');
  });
});
