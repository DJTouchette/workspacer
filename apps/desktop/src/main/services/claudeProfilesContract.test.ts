// The claude.profiles.* contract, shared with services/hub/cmd/brain/profiles.go.
//
// claude.profiles.* is answered by the BRAIN for every web / mobile / remote /
// MCP client and by this service over IPC, and the two disagreed about what the
// store even contains: a synthetic Default row on one side that was listed but
// never written (and that its own update then refused to find), normalized list
// fields on one side and the raw file on the other, and an `add` on a fresh
// config dir that minted isDefault:true on one provider and false on the other.
//
// Fixture: contracts/claude-profiles-cases.json.
// Twin loader: TestClaudeProfilesContractCases in cmd/brain.
//
// The service is a module singleton whose constructor reads and writes the
// config dir, so each case re-imports it against a fresh sandbox.

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

interface Profile {
  id: string;
  name: string;
  configDir: string;
  extraArgs?: string[];
  mcpItemIds?: string[];
  isDefault: boolean;
}
interface Fixture {
  list: {
    name: string;
    file: Profile[] | null;
    expectedList: Profile[];
    expectedFile: Profile[];
    why: string;
  }[];
  add: {
    name: string;
    file: Profile[] | null;
    add: {
      name: string;
      configDir: string;
      extraArgs: string[] | null;
      mcpItemIds: string[] | null;
    };
    expectedAdded: Omit<Profile, 'id'>;
    expectedFileIds: string[];
    why: string;
  }[];
  mutate: {
    name: string;
    file: Profile[] | null;
    updateId?: string;
    update?: { name: string };
    expectFound?: boolean;
    removeId?: string;
    expectedFileIds?: string[];
    why: string;
  }[];
}

const state = vi.hoisted(() => ({ dir: '' }));
vi.mock('./configService', () => ({ getConfigDir: () => state.dir }));

const fixture: Fixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/claude-profiles-cases.json'),
    'utf-8',
  ),
);

const profilesPath = (): string => path.join(state.dir, 'claude-profiles.json');

function seed(file: Profile[] | null): void {
  state.dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-prof-')));
  if (file !== null) fs.writeFileSync(profilesPath(), JSON.stringify({ profiles: file }));
  // The service is a singleton constructed at import time, so the module cache
  // has to be dropped for the constructor to run against this sandbox.
  vi.resetModules();
}

const onDisk = (): Profile[] =>
  fs.existsSync(profilesPath())
    ? (JSON.parse(fs.readFileSync(profilesPath(), 'utf-8')).profiles as Profile[])
    : [];

let sandboxes: string[] = [];
beforeEach(() => {
  sandboxes = [];
});
afterEach(() => {
  for (const d of sandboxes) fs.rmSync(d, { recursive: true, force: true });
});

async function service(): Promise<typeof import('./claudeProfiles').claudeProfiles> {
  const mod = await import('./claudeProfiles');
  sandboxes.push(state.dir);
  return mod.claudeProfiles;
}

describe('contracts/claude-profiles-cases.json — list', () => {
  for (const c of fixture.list) {
    it(c.name, async () => {
      seed(c.file);
      const svc = await service();
      expect(svc.getProfiles(), c.why).toEqual(c.expectedList);
      expect(onDisk(), 'the file on disk must match too').toEqual(c.expectedFile);
    });
  }
});

describe('contracts/claude-profiles-cases.json — add', () => {
  for (const c of fixture.add) {
    it(c.name, async () => {
      seed(c.file);
      const svc = await service();
      const added = svc.addProfile(
        c.add.name,
        c.add.configDir,
        c.add.extraArgs ?? [],
        c.add.mcpItemIds ?? [],
      );
      expect(added.id).toBeTruthy();
      const { id: _id, ...rest } = added;
      expect(rest, c.why).toEqual(c.expectedAdded);
      expect(onDisk().map((p) => p.id)).toEqual(
        c.expectedFileIds.map((x) => (x === '<added>' ? added.id : x)),
      );
    });
  }
});

describe('contracts/claude-profiles-cases.json — update/remove', () => {
  for (const c of fixture.mutate) {
    it(c.name, async () => {
      seed(c.file);
      const svc = await service();
      if (c.updateId) {
        svc.getProfiles(); // the caller has listed first, as a client would
        const got = svc.updateProfile(c.updateId, { name: c.update!.name });
        if (c.expectFound) {
          expect(
            got,
            `update(${c.updateId}) must find an id list() returned — ${c.why}`,
          ).not.toBeNull();
          expect(got!.name).toBe(c.update!.name);
        }
      }
      if (c.removeId) svc.removeProfile(c.removeId);
      if (c.expectedFileIds) {
        expect(
          onDisk().map((p) => p.id),
          c.why,
        ).toEqual(c.expectedFileIds);
      }
    });
  }
});
