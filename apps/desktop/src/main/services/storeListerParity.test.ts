/**
 * layouts.list / sessions.list must survive a store row whose scalar is not a
 * string. Valid YAML timestamps from older Rust saves retain their date/order;
 * other non-string scalars retain the legacy empty-sort-key behavior.
 *
 * `(b.createdAt || '').localeCompare(...)` is a METHOD CALL on a value that came
 * out of YAML. `createdAt: 5` is a number and `createdAt: 2026-03-01T00:00:00.000Z`
 * (unquoted — the form a hand edit produces) is parsed to a Date by js-yaml 4, so
 * as soon as V8's insertion sort put that row in the `b` position the comparator
 * threw. The throw landed in the function-level try/catch, whose catch returns
 * `[]` — so BOTH lists came back EMPTY, taking every well-formed layout and
 * session with them, while the brain listed them all. That is the desktop's own
 * Sessions/Layouts UI as well as the two bus capabilities, and it is remotely
 * reachable: <configDir>/layouts and <configDir>/sessions are configStoreRoots,
 * so writing the file is an ordinary permitted fs.write.
 *
 * It is readdir-order dependent, which is why it survived every existing test —
 * a poisoned row that sorts LAST is never the `b` argument.
 *
 * Historical Go control: TestStoreListersSurviveANonStringScalar. Valid dates now
 * preserve existing Rust saved state instead of becoming undated entries.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => {
  const nodeFs = require('fs') as typeof import('fs');
  const nodeOs = require('os') as typeof import('os');
  const nodePath = require('path') as typeof import('path');
  return { configDir: nodeFs.mkdtempSync(nodePath.join(nodeOs.tmpdir(), 'wks-store-cfg-')) };
});
vi.mock('./configService', () => ({ getConfigDir: () => h.configDir }));
vi.mock('./claudemonSessionClient', () => ({ claudemonSessionClient: {} }));
vi.mock('./hubClient', () => ({ publishToHub: () => {} }));

import { layoutService } from './layoutService';
import { sessionService } from './sessionService';

beforeEach(() => {
  h.configDir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-store-cfg-')));
});
afterEach(() => {
  fs.rmSync(h.configDir, { recursive: true, force: true });
});

const REAL_STAMPS = [
  "'2026-03-01T00:00:00.000Z'",
  "'2026-02-01T00:00:00.000Z'",
  "'2026-01-01T00:00:00.000Z'",
];

// Numbers/containers are not timestamps. Valid implicit dates from older Rust
// saves and explicit YAML timestamp values are supported upgrade inputs.
const TIMESTAMP_INPUTS = [
  { name: 'number', yaml: '5', date: false },
  { name: 'mapping', yaml: '{ unexpected: true }', date: false },
  { name: 'array', yaml: '[unexpected]', date: false },
  { name: 'old Rust implicit date', yaml: '2026-03-01T00:00:00.000Z', date: true },
  { name: 'explicit YAML date', yaml: "!!timestamp '2026-03-01T00:00:00.000Z'", date: true },
];

describe.each(TIMESTAMP_INPUTS)('a store row with $name', ({ yaml: scalar, date }) => {
  it('does not empty layouts.list', () => {
    const dir = path.join(h.configDir, 'layouts');
    fs.mkdirSync(dir, { recursive: true });
    REAL_STAMPS.forEach((ts, i) => {
      const name = `real${i + 1}`;
      fs.writeFileSync(
        path.join(dir, `${name}.yaml`),
        `id: ${name}\nname: ${name}\ncreatedAt: ${ts}\nagents: []\n`,
      );
    });
    // "aaa" so it lands FIRST in readdir order — which is what puts it in the
    // `b` position of the comparator.
    fs.writeFileSync(
      path.join(dir, 'aaa.yaml'),
      `id: aaa\nname: Aaa\ncreatedAt: ${scalar}\nagents: []\n`,
    );

    const got = layoutService.list().map((l) => l.id);
    expect(got).toEqual(
      !date ? ['real1', 'real2', 'real3', 'aaa'] : ['aaa', 'real1', 'real2', 'real3'],
    );
    if (date) expect(layoutService.list()[0].createdAt).toBe('2026-03-01T00:00:00.000Z');
  });

  it('does not empty sessions.list', () => {
    const dir = path.join(h.configDir, 'sessions');
    fs.mkdirSync(dir, { recursive: true });
    REAL_STAMPS.forEach((ts, i) => {
      const name = `real${i + 1}`;
      fs.writeFileSync(path.join(dir, `${name}.yaml`), `name: ${name}\ntimestamp: ${ts}\n`);
    });
    fs.writeFileSync(path.join(dir, 'aaa.yaml'), `name: aaa\ntimestamp: ${scalar}\n`);

    const got = sessionService.listSessions().map((s) => s.name);
    expect(got).toEqual(
      !date ? ['real1', 'real2', 'real3', 'aaa'] : ['aaa', 'real1', 'real2', 'real3'],
    );
    if (date) expect(sessionService.listSessions()[0].timestamp).toBe('2026-03-01T00:00:00.000Z');
  });
});

// ORDER, not only survival: `timestamp` is a coerced string now, so localeCompare
// no longer throws — it just puts the list in a different order than the Go
// brain's `out[i].Timestamp > out[j].Timestamp`. Same for layouts.
// Fixture: contracts/provider-parity-cases.json.
describe('store list ordering matches the Go provider', () => {
  const fixture = JSON.parse(
    fs.readFileSync(
      path.resolve(__dirname, '../../../../../contracts/provider-parity-cases.json'),
      'utf-8',
    ),
  ) as { order: { input: string[]; expected: string[] }[] };
  const c = fixture.order[0];

  it('sessions.list sorts by timestamp byte-wise, newest first', () => {
    const dir = path.join(h.configDir, 'sessions');
    fs.mkdirSync(dir, { recursive: true });
    c.input.forEach((ts, i) => {
      fs.writeFileSync(path.join(dir, `s${i}.yaml`), `name: s${i}\ntimestamp: '${ts}'\n`);
    });
    const byTimestamp = new Map(c.input.map((ts, i) => [ts, `s${i}`]));
    expect(sessionService.listSessions().map((s) => s.name)).toEqual(
      [...c.expected].reverse().map((ts) => byTimestamp.get(ts)),
    );
  });

  it('layouts.list sorts by createdAt byte-wise, newest first', () => {
    const dir = path.join(h.configDir, 'layouts');
    fs.mkdirSync(dir, { recursive: true });
    c.input.forEach((ts, i) => {
      fs.writeFileSync(
        path.join(dir, `l${i}.yaml`),
        `id: l${i}\nname: l${i}\ncreatedAt: '${ts}'\nagents: []\n`,
      );
    });
    const byCreatedAt = new Map(c.input.map((ts, i) => [ts, `l${i}`]));
    expect(layoutService.list().map((l) => l.id)).toEqual(
      [...c.expected].reverse().map((ts) => byCreatedAt.get(ts)),
    );
  });
});
