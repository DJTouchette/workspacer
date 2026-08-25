/**
 * No spec may be orphaned by the project split.
 *
 * `playwright.config.ts` divides `tests/e2e` into projects so CI can run the
 * cheap ones (see its header for why). The cost of that is a new failure mode:
 * a spec matching no project is never run, by anyone, and a spec nobody runs is
 * indistinguishable from a spec that passes. That is precisely how
 * `mobileClient.test.ts` sat red for two days.
 *
 * So the split is asserted, not assumed. Every `*.test.ts` in this directory
 * must be claimed by exactly one project — one, because two would run it twice
 * and double every hub it boots.
 */
import { test, expect } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';
import config from '../../playwright.config';

test('every e2e spec is claimed by exactly one project', () => {
  const dir = __dirname;
  const specs = fs.readdirSync(dir).filter((f) => f.endsWith('.test.ts'));
  expect(specs.length, 'no specs found — this guard would be vacuous').toBeGreaterThan(0);

  const projects = (config.projects ?? []) as Array<{ name?: string; testMatch?: unknown }>;
  expect(projects.length, 'no projects configured — this guard would be vacuous').toBeGreaterThan(
    0,
  );

  const orphans: string[] = [];
  const doubled: string[] = [];
  for (const spec of specs) {
    const full = path.join(dir, spec);
    const owners = projects
      .filter((p) => p.testMatch instanceof RegExp && (p.testMatch as RegExp).test(full))
      .map((p) => p.name);
    if (owners.length === 0) orphans.push(spec);
    if (owners.length > 1) doubled.push(`${spec} → ${owners.join(', ')}`);
  }

  expect(
    orphans,
    'these specs match no Playwright project, so nothing runs them — add a project or rename to a ' +
      'claimed prefix (app*/mobile*), and wire it into .github/workflows/ci.yml if it is cheap',
  ).toEqual([]);
  expect(doubled, 'these specs are claimed by more than one project').toEqual([]);
});
