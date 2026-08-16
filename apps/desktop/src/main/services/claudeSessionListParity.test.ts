/**
 * `claude.sessionsForDir` derives the resume sessionId from the transcript
 * filename, and the two providers did it differently: `file.replace('.jsonl','')`
 * removes the FIRST occurrence ANYWHERE, while the Go twin (discovery.go) uses
 * strings.TrimSuffix.
 *
 *   'a.jsonl.b.jsonl'      → 'a.b.jsonl'   here vs 'a.jsonl.b'   there
 *   '.jsonlagent-x.jsonl'  → 'agent-x.jsonl' here — which then matches the
 *                            startsWith('agent-') subagent filter and DROPS the
 *                            row, so the same call returned 2 rows from one
 *                            provider and 3 from the other.
 *
 * TWIN: the `suffix` block of contracts/provider-parity-cases.json, loaded by
 * both sides.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({ home: '' }));
vi.mock('os', async (importOriginal) => {
  const actual = await importOriginal<typeof import('os')>();
  return { ...actual, default: { ...actual, homedir: () => h.home }, homedir: () => h.home };
});

import { listClaudeSessionsForDir } from './claudeSessionList';

let projectDir: string;
const CWD = '/proj/fix';

beforeEach(() => {
  h.home = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-csl-home-')));
  projectDir = path.join(h.home, '.claude', 'projects', '-proj-fix');
  fs.mkdirSync(projectDir, { recursive: true });
});
afterEach(() => {
  fs.rmSync(h.home, { recursive: true, force: true });
});

describe('claude.sessionsForDir id derivation', () => {
  it('trims the .jsonl SUFFIX, not the first occurrence', async () => {
    for (const name of ['plain.jsonl', 'a.jsonl.b.jsonl', '.jsonlagent-x.jsonl']) {
      fs.writeFileSync(path.join(projectDir, name), '{"type":"user"}\n');
    }
    const ids = (await listClaudeSessionsForDir(CWD)).map((s) => s.sessionId)
      .sort();
    expect(ids).toEqual(['.jsonlagent-x', 'a.jsonl.b', 'plain']);
  });

  it('still drops a real subagent transcript', async () => {
    for (const name of ['plain.jsonl', 'agent-sub.jsonl']) {
      fs.writeFileSync(path.join(projectDir, name), '{"type":"user"}\n');
    }
    expect((await listClaudeSessionsForDir(CWD)).map((s) => s.sessionId)).toEqual(['plain']);
  });
});
