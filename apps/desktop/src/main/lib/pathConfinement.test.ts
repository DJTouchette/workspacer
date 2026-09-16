import { describe, expect, it, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

vi.mock('../services/configService', () => ({ getConfigDir: () => path.join(os.tmpdir(), 'wks') }));

import {
  MAX_LINK_HOPS,
  assertPathAllowed,
  assertPathContained,
  canonicalizePath,
  containsCanonical,
} from './pathConfinement';

interface Tree {
  dirs?: string[];
  symlinks?: Record<string, string>;
}

interface ActivePathCase {
  name: string;
  group: 'ambient-canonicalization' | 'selected-object';
  expect: 'allow' | 'deny';
  why: string;
  roots?: string[];
  target: string;
  resolvesTo?: string;
  needsSymlinks?: boolean;
  tree?: Tree;
}

interface Fixture {
  maxLinkHops: number;
  cases: ActivePathCase[];
}

const fixture: Fixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf8',
  ),
);

const substitute = (value: string, sandbox: string): string =>
  value
    .replaceAll('${SANDBOX}', sandbox)
    .replaceAll('${ROOT}', path.join(sandbox, 'root'))
    .replaceAll('${OUTSIDE}', path.join(sandbox, 'outside'));

describe('active path contract', () => {
  it('keeps a non-vacuous cross-language corpus and hop limit', () => {
    expect(fixture.cases.length).toBeGreaterThanOrEqual(7);
    expect(fixture.maxLinkHops).toBe(MAX_LINK_HOPS);
  });

  for (const testCase of fixture.cases) {
    it(testCase.name, () => {
      const sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-path-')));
      try {
        fs.mkdirSync(path.join(sandbox, 'root'), { recursive: true });
        fs.mkdirSync(path.join(sandbox, 'outside'), { recursive: true });
        for (const dir of testCase.tree?.dirs ?? []) {
          fs.mkdirSync(path.join(sandbox, dir), { recursive: true });
        }
        for (const [link, target] of Object.entries(testCase.tree?.symlinks ?? {})) {
          const linkPath = path.join(sandbox, link);
          fs.mkdirSync(path.dirname(linkPath), { recursive: true });
          try {
            fs.symlinkSync(path.join(sandbox, target), linkPath, 'junction');
          } catch (error) {
            if (testCase.needsSymlinks) return;
            throw error;
          }
        }
        const roots = (testCase.roots ?? []).map((value) => substitute(value, sandbox));
        const target = substitute(testCase.target, sandbox);
        const run = (): string =>
          testCase.group === 'selected-object'
            ? assertPathContained('contract', target, roots)
            : assertPathAllowed('contract', target, roots);
        if (testCase.expect === 'deny') {
          expect(run).toThrow();
        } else {
          expect(run()).toBe(path.normalize(substitute(testCase.resolvesTo as string, sandbox)));
        }
      } finally {
        fs.rmSync(sandbox, { recursive: true, force: true });
      }
    });
  }
});

describe('ambient paths and selected objects', () => {
  it('does not treat former workspace roots or secret names as authorization', () => {
    const selected = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-selected-')));
    const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-outside-')));
    try {
      const target = path.join(outside, '.bus-token');
      expect(assertPathAllowed('fs.read', target, [selected])).toBe(canonicalizePath(target));
      expect(() => assertPathContained('library.read', target, [selected])).toThrow(
        /outside the selected object/,
      );
    } finally {
      fs.rmSync(selected, { recursive: true, force: true });
      fs.rmSync(outside, { recursive: true, force: true });
    }
  });

  it('uses a separator boundary for selected-object containment', () => {
    const selected = path.join(os.tmpdir(), 'wks-boundary-repo');
    expect(containsCanonical(selected, path.join(selected, 'file'))).toBe(true);
    expect(
      containsCanonical(selected, path.join(os.tmpdir(), 'wks-boundary-repository', 'file')),
    ).toBe(false);
  });
});
