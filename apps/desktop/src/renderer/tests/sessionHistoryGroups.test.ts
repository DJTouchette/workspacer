import { describe, it, expect } from 'vitest';
import {
  buildHistoryGroups,
  syntheticDaemonRow,
  OTHER_GROUP_KEY,
} from '../src/lib/sessionHistoryGroups';
import type { RecentAgentSession } from '../../main/shared/ipcTypes';

/**
 * The Sessions pane's grouping: transcript listings are the primary content
 * per project, daemon rows fill in what transcripts can't know (managed
 * providers, unregistered directories), and a session both sources know must
 * come out as ONE row that keeps the best of each.
 */

const T0 = Date.parse('2026-08-01T12:00:00Z');

const transcript = (sessionId: string, summary: string, minsAgo = 0) => ({
  sessionId,
  timestamp: new Date(T0 - minsAgo * 60_000).toISOString(),
  summary,
});

const daemon = (over: Partial<RecentAgentSession>): RecentAgentSession => ({
  sessionId: 'd1',
  provider: 'claude',
  cwd: '/w/app',
  mode: 'stopped',
  transport: 'pty',
  archived: false,
  updatedAt: 0,
  startedAt: 0,
  name: '',
  title: '',
  model: '',
  costUSD: 0,
  ...over,
});

describe('buildHistoryGroups', () => {
  it('groups transcript sessions under their project, newest first', () => {
    const groups = buildHistoryGroups(
      ['/w/app'],
      { '/w/app': [transcript('s-old', 'older', 60), transcript('s-new', 'newer', 5)] },
      [],
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].dir).toBe('/w/app');
    expect(groups[0].rows.map((r) => r.sessionId)).toEqual(['s-new', 's-old']);
    expect(groups[0].rows[0].label).toBe('newer');
    expect(groups[0].rows[0].provider).toBe('claude');
  });

  it('folds a session both sources know into one row — transcript label, daemon facts', () => {
    const d = daemon({
      sessionId: 's1',
      model: 'opus',
      archived: true,
      updatedAt: T0 + 60_000,
      name: 'app', // spawn-default (dir basename) — NOT an explicit name
    });
    const groups = buildHistoryGroups(
      ['/w/app'],
      { '/w/app': [transcript('s1', 'fix the flaky test')] },
      [d],
    );
    expect(groups[0].rows).toHaveLength(1);
    const row = groups[0].rows[0];
    expect(row.label).toBe('fix the flaky test');
    expect(row.model).toBe('opus');
    expect(row.archived).toBe(true);
    // The fresher of the two timestamps wins.
    expect(row.updatedAt).toBe(T0 + 60_000);
    // Resume goes through the daemon's row so transport/model survive.
    expect(row.daemon).toBe(d);
  });

  it('prefers an explicitly-given agent name over the transcript summary', () => {
    const groups = buildHistoryGroups(
      ['/w/app'],
      { '/w/app': [transcript('s1', 'first user message')] },
      [daemon({ sessionId: 's1', name: 'release captain' })],
    );
    expect(groups[0].rows[0].label).toBe('release captain');
  });

  it('keeps daemon-only rows in their project group (managed providers)', () => {
    const groups = buildHistoryGroups(
      ['/w/app'],
      { '/w/app': [transcript('s1', 'claude work')] },
      [daemon({ sessionId: 'c1', provider: 'codex', title: 'codex work', updatedAt: T0 + 1 })],
    );
    expect(groups[0].rows.map((r) => r.sessionId)).toEqual(['c1', 's1']);
    expect(groups[0].rows[0].provider).toBe('codex');
    expect(groups[0].rows[0].label).toBe('codex work');
  });

  it('puts daemon rows from unregistered directories in the trailing catch-all', () => {
    const groups = buildHistoryGroups(['/w/app'], { '/w/app': [transcript('s1', 'x')] }, [
      daemon({ sessionId: 'stray', cwd: '/tmp/scratch', title: 'one-off' }),
    ]);
    expect(groups.map((g) => g.key)).toEqual(['/w/app', OTHER_GROUP_KEY]);
    expect(groups[1].dir).toBe('');
    expect(groups[1].rows[0].sessionId).toBe('stray');
  });

  it('hides excluded sessions (in-layout or live elsewhere) and empty groups', () => {
    const groups = buildHistoryGroups(
      ['/w/app', '/w/empty'],
      { '/w/app': [transcript('s1', 'x'), transcript('s2', 'y')], '/w/empty': [] },
      [daemon({ sessionId: 's3', cwd: '/tmp/other' })],
      ['s1', 's3'],
    );
    // /w/empty has nothing; the catch-all lost its only row to the exclusion.
    expect(groups.map((g) => g.key)).toEqual(['/w/app']);
    expect(groups[0].rows.map((r) => r.sessionId)).toEqual(['s2']);
  });

  it('normalizes dir spellings when matching daemon rows to a project', () => {
    // Daemon rows carry the cwd as the agent saw it — a trailing slash must
    // still land in the project's group, not the catch-all.
    const groups = buildHistoryGroups(['/w/app'], {}, [
      daemon({ sessionId: 'c1', cwd: '/w/app/', provider: 'codex', title: 't' }),
    ]);
    expect(groups.map((g) => g.key)).toEqual(['/w/app']);
  });
});

describe('syntheticDaemonRow', () => {
  it('shapes a transcript-only row like a legacy daemon row', () => {
    const groups = buildHistoryGroups(
      ['/w/app'],
      { '/w/app': [transcript('s1', 'the summary')] },
      [],
    );
    const wire = syntheticDaemonRow(groups[0].rows[0]);
    expect(wire.sessionId).toBe('s1');
    expect(wire.provider).toBe('claude');
    expect(wire.cwd).toBe('/w/app');
    expect(wire.mode).toBe('stopped');
    // 'pty' reads as "no recorded transport choice" on the resume path, so
    // the config default decides — same as a legacy daemon row.
    expect(wire.transport).toBe('pty');
    expect(wire.title).toBe('the summary');
  });
});
