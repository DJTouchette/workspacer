// Which session rows put a cwd in the fs.* allow-list — the root SUPPLY half of
// path containment, held to the same shared fixture as the containment rule
// itself.
//
// contracts/path-containment-cases.json answers "is this path inside a root".
// This block answers the other half, and the two providers disagreed on it
// completely: the brain's agentCwds() has filtered on snapshotLive since it was
// written, and this side's workspaceRoots() iterated every snapshot with no
// state test at all. So one session row granted an fs root on one provider and
// was refused by the other, forever — and this store's only removal path is a
// 30-second timer armed by a SessionEnd hook, so a PTY killed without one
// (SIGKILL, crash, OOM) kept `status: 'active'` and kept its directory granted
// for the life of the app process. git.diff, fs.readImage, fs.watch and
// fs.unwatch are answered HERE even under the default catalog delegation, so
// that was the shipping configuration.
//
// TWIN: services/hub/cmd/brain/visibility_test.go TestAgentCwdLivenessContractCases.

import { describe, it, expect } from 'vitest';
import * as fs from 'fs';
import * as path from 'path';
import { snapshotGrantsFsRoot } from './snapshotLiveness';

interface LivenessCase {
  name: string;
  snapshot: Record<string, unknown>;
  live: boolean;
  why: string;
}

// apps/desktop/src/main/lib/ → five levels below the repo root.
const fixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
) as { agentCwdLiveness: { cases: LivenessCase[] } };

describe('agent-cwd liveness — cross-language contract', () => {
  const cases = fixture.agentCwdLiveness?.cases ?? [];

  it('the block still carries cases, and both verdicts', () => {
    // A silently emptied block agrees with everything, and a block that carries
    // only one verdict is passed by a copy that answers a constant.
    expect(cases.length).toBeGreaterThanOrEqual(9);
    expect(cases.some((c) => c.live)).toBe(true);
    expect(cases.some((c) => !c.live)).toBe(true);
  });

  for (const c of cases) {
    it(c.name, () => {
      expect(snapshotGrantsFsRoot(c.snapshot), c.why).toBe(c.live);
    });
  }
});
