// The claude.listModels contract, shared with services/hub/cmd/brain/models.go.
//
// claude.listModels is a CATALOG method: the brain answers it for every web,
// mobile, remote and MCP client under the default DELEGATE_CATALOG_TO_BRAIN,
// while this copy serves the desktop's own IPC path. The two returned different
// alias sets (four rows vs six — opus[1m] and sonnet[1m] were unreachable from
// the web picker), different labels, no context badges on one side and no
// defaultPermissionMode on one side, and each suite asserted its own answer.
//
// Fixture: contracts/claude-model-catalog-cases.json.
// Twin loader: TestClaudeModelCatalogContractCases in cmd/brain.

import { describe, it, expect, vi } from 'vitest';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import * as fs from 'fs';
import * as path from 'path';

interface Case {
  name: string;
  config: {
    defaultModel: string;
    contextWindow: number | null;
    skipPermissionsDefault: boolean;
    defaultPermissionMode: string;
    seenModels: string[];
  };
  live: string[];
  expected: unknown;
  why: string;
}

const state = vi.hoisted(() => ({
  cfg: {} as Record<string, unknown>,
  snapshots: [] as { usage?: { model?: string } }[],
}));
vi.mock('./configService', () => ({ configService: { getConfig: () => state.cfg } }));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { getAllSnapshots: () => state.snapshots },
}));

const { listClaudeModels } = await import('./claudeModels');

const fixture: { cases: Case[] } = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/claude-model-catalog-cases.json'),
    'utf-8',
  ),
);

describe('contracts/claude-model-catalog-cases.json', () => {
  it('has cases', () => {
    expect(fixture.cases.length).toBeGreaterThan(0);
  });

  const tally = new SweepTally();
  for (const c of fixture.cases) {
    it(c.name, () => {
      tally.ran('other');
      state.cfg = { claude: { ...c.config } };
      state.snapshots = c.live.map((model) => ({ usage: { model } }));
      // Compared as JSON so field ORDER is pinned too — the Go twin marshals its
      // struct and a reordered reply is a different wire answer.
      expect(JSON.parse(JSON.stringify(listClaudeModels())), c.why).toEqual(c.expected);
    });
  }
  itSweptTheWholeCorpus(tally, 'the claude-model-catalog corpus', 7, { allow: 0, deny: 0 });
});
