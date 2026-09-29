/** Cross-plane registry obligations of the retired Go brain parameter scanner.
 * Typed Rust payload/helper tracing lives in tools/capability-source-check. */
import { describe, it, expect } from 'vitest';
import path from 'path';
import reference from '../../../../../contracts/backend-capabilities.json';
import {
  inertMethods,
  classified,
  missingSpec,
  pathParameters,
} from '../../../tests/support/capabilityParameters';
import {
  desktopRegistrations,
  registrations,
  rustSources,
} from '../../../tests/support/capabilitySource';
const ROOT = path.resolve(__dirname, '../../../../..');
function registered(): Set<string> {
  return new Set([
    ...registrations(rustSources(ROOT)).map((row) => row.method),
    ...desktopRegistrations(ROOT).map((row) => row.method),
  ]);
}
function inertRegistered(names: Set<string>, inert = inertMethods): void {
  if (names.size < 150) throw Error('registry population collapsed');
  for (const name of Object.keys(inert))
    if (!names.has(name)) throw Error('phantom inert method ' + name);
}
describe('brain parameter registry closure', () => {
  it('requires every inert record to have an actual Rust or retained desktop registration', () =>
    expect(() => inertRegistered(registered())).not.toThrow());
  it('retains full/catalog classification and deliberately ambient spawn cwd', () => {
    const methods = new Set([...reference.full, ...reference.catalog]);
    expect(methods.size).toBeGreaterThan(40);
    for (const method of methods) {
      expect(classified(method), method).toBe(true);
      expect(missingSpec(method), method).toBe(false);
    }
    expect(pathParameters['agents.spawn']).toBeUndefined();
    expect(missingSpec('agents.spawn')).toBe(false);
  });
  it('rejects phantom inert entries and deletion of a real provider registration', () => {
    expect(() =>
      inertRegistered(registered(), {
        ...inertMethods,
        'fs.exfiltrate': 'no params, supposedly safe',
      }),
    ).toThrow('phantom');
    const names = registered();
    names.delete('providers.checkAll');
    expect(() => inertRegistered(names)).toThrow('providers.checkAll');
    expect(() => inertRegistered(new Set())).toThrow('population');
  });
});
