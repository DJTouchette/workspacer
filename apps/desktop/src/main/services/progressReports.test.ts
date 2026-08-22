/**
 * report_progress: the worker-initiated half of gap #8. These tests pin the
 * bounds that make an unsolicited, host-refusing channel trustworthy — the
 * caller identity can never be spoofed or guessed, the recipient can never be
 * anyone but the caller's own parent, and every refusal is loud (a worker that
 * believes it reported and did not is the exact failure this exists to
 * prevent).
 */
import { describe, it, expect, vi } from 'vitest';
import {
  ProgressReports,
  flattenNote,
  NOTE_MAX,
  MIN_INTERVAL_MS,
  MAX_REPORTS,
} from './progressReports';
import { parseFleetMessage } from '../shared/fleetMessages';

const session = (over: Partial<Record<string, unknown>> = {}) => ({
  sessionId: 'w1',
  cwd: '/home/u/Work/alpha',
  label: 'alpha: ship',
  status: 'active',
  parentSessionId: 'mgr',
  ...over,
});

const mgr = (over: Partial<Record<string, unknown>> = {}) => ({
  sessionId: 'mgr',
  label: 'Fleet Manager',
  status: 'active',
  ...over,
});

function rig(sessions: ReturnType<typeof session>[]) {
  const deliver = vi.fn().mockResolvedValue(undefined);
  const reports = new ProgressReports(deliver, () => sessions as any);
  return { deliver, reports };
}

describe('flattenNote', () => {
  it('collapses whitespace (including newlines) to single spaces and trims', () => {
    expect(flattenNote('  finished\n  phase 1,\t moving on  ')).toBe('finished phase 1, moving on');
  });
});

describe('ProgressReports.report', () => {
  it('refuses a caller with no session identity, without guessing a recipient', async () => {
    const { reports } = rig([session(), mgr()]);
    await expect(reports.report({ note: 'hi' })).rejects.toThrow(/could not identify your session/);
  });

  it('refuses an empty note', async () => {
    const { reports } = rig([session(), mgr()]);
    await expect(reports.report({ callerSessionId: 'w1', note: '   ' })).rejects.toThrow(
      /non-empty note/,
    );
  });

  it('refuses a note over NOTE_MAX', async () => {
    const { reports } = rig([session(), mgr()]);
    await expect(
      reports.report({ callerSessionId: 'w1', note: 'x'.repeat(NOTE_MAX + 1) }),
    ).rejects.toThrow(/the limit is/);
  });

  it('refuses a caller that is not a tracked session', async () => {
    const { reports } = rig([mgr()]);
    await expect(reports.report({ callerSessionId: 'ghost', note: 'hi' })).rejects.toThrow(
      /not a tracked session/,
    );
  });

  it('refuses a caller with no parent — cannot report to nobody', async () => {
    const { reports } = rig([session({ parentSessionId: undefined }), mgr()]);
    await expect(reports.report({ callerSessionId: 'w1', note: 'hi' })).rejects.toThrow(
      /no parent session/,
    );
  });

  it('refuses a caller whose parent is itself', async () => {
    const { reports } = rig([session({ sessionId: 'w1', parentSessionId: 'w1' })]);
    await expect(reports.report({ callerSessionId: 'w1', note: 'hi' })).rejects.toThrow(
      /no parent session/,
    );
  });

  it('refuses when the parent has ended', async () => {
    const { reports } = rig([session(), mgr({ status: 'ended' })]);
    await expect(reports.report({ callerSessionId: 'w1', note: 'hi' })).rejects.toThrow(
      /has ended/,
    );
  });

  it('delivers to the parent, never a caller-named session', async () => {
    const { deliver, reports } = rig([session(), mgr()]);
    const result = await reports.report({ callerSessionId: 'w1', note: 'finished phase 1' });
    expect(result).toEqual({ deliveredTo: 'mgr' });
    expect(deliver).toHaveBeenCalledTimes(1);
    expect(deliver).toHaveBeenCalledWith('mgr', expect.any(String));
  });

  it('delivers a well-formed progress fleet message round-tripping the note', async () => {
    const { deliver, reports } = rig([session(), mgr()]);
    await reports.report({ callerSessionId: 'w1', note: 'the approach you gave me is wrong' });
    const text = deliver.mock.calls[0][1] as string;
    const parsed = parseFleetMessage(text);
    expect(parsed?.kind).toBe('progress');
    expect(parsed?.entries).toEqual([
      expect.objectContaining({
        sessionId: 'w1',
        note: 'the approach you gave me is wrong',
      }),
    ]);
  });

  it('round-trips needsDecision', async () => {
    const { deliver, reports } = rig([session(), mgr()]);
    await reports.report({ callerSessionId: 'w1', note: 'stuck', needsDecision: true });
    const parsed = parseFleetMessage(deliver.mock.calls[0][1] as string);
    expect(parsed?.entries[0]).toMatchObject({ needsDecision: true });
  });

  it('falls back to a cwd-basename label when the session has none', async () => {
    const { deliver, reports } = rig([session({ label: undefined }), mgr()]);
    await reports.report({ callerSessionId: 'w1', note: 'hi' });
    const parsed = parseFleetMessage(deliver.mock.calls[0][1] as string);
    expect(parsed?.entries[0]).toMatchObject({ label: 'alpha' });
  });

  it('refuses a second identical note in a row without delivering it again', async () => {
    const { deliver, reports } = rig([session(), mgr()]);
    await reports.report({ callerSessionId: 'w1', note: 'same note' }, 0);
    await expect(
      reports.report({ callerSessionId: 'w1', note: 'same note' }, 1000),
    ).rejects.toThrow(/NOT delivered again/);
    expect(deliver).toHaveBeenCalledTimes(1);
  });

  it('rate-limits reports from one worker to MIN_INTERVAL_MS apart', async () => {
    const { deliver, reports } = rig([session(), mgr()]);
    await reports.report({ callerSessionId: 'w1', note: 'first' }, 0);
    await expect(
      reports.report({ callerSessionId: 'w1', note: 'second' }, MIN_INTERVAL_MS - 1),
    ).rejects.toThrow(/limited to one per/);
    expect(deliver).toHaveBeenCalledTimes(1);
    await reports.report({ callerSessionId: 'w1', note: 'second' }, MIN_INTERVAL_MS);
    expect(deliver).toHaveBeenCalledTimes(2);
  });

  it('caps a session at MAX_REPORTS lifetime, even spaced well apart', async () => {
    const { deliver, reports } = rig([session(), mgr()]);
    for (let i = 0; i < MAX_REPORTS; i++) {
      await reports.report({ callerSessionId: 'w1', note: `update ${i}` }, i * MIN_INTERVAL_MS);
    }
    expect(deliver).toHaveBeenCalledTimes(MAX_REPORTS);
    await expect(
      reports.report({ callerSessionId: 'w1', note: 'one more' }, MAX_REPORTS * MIN_INTERVAL_MS),
    ).rejects.toThrow(/the limit for one session/);
    expect(deliver).toHaveBeenCalledTimes(MAX_REPORTS);
  });

  it('charges the budget even when delivery fails, so a retry loop still hits the cap', async () => {
    const deliver = vi.fn().mockRejectedValue(new Error('network down'));
    const reports = new ProgressReports(deliver, () => [session(), mgr()] as any);
    await expect(reports.report({ callerSessionId: 'w1', note: 'hi' }, 0)).rejects.toThrow(
      /network down/,
    );
    await expect(
      reports.report({ callerSessionId: 'w1', note: 'hi again' }, MIN_INTERVAL_MS - 1),
    ).rejects.toThrow(/limited to one per/);
  });

  it('tracks budgets per-session independently', async () => {
    const { deliver, reports } = rig([
      session({ sessionId: 'w1' }),
      session({ sessionId: 'w2' }),
      mgr(),
    ]);
    await reports.report({ callerSessionId: 'w1', note: 'from w1' }, 0);
    await reports.report({ callerSessionId: 'w2', note: 'from w2' }, 0);
    expect(deliver).toHaveBeenCalledTimes(2);
  });

  it('reset() clears budgets so a fresh report is accepted immediately', async () => {
    const { deliver, reports } = rig([session(), mgr()]);
    await reports.report({ callerSessionId: 'w1', note: 'first' }, 0);
    reports.reset();
    await reports.report({ callerSessionId: 'w1', note: 'second' }, 1);
    expect(deliver).toHaveBeenCalledTimes(2);
  });
});
