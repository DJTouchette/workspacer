/**
 * The hub-owned "too many sessions" prompt (docs/session-roster.md §6): shown
 * only when the hub suggests it AND serves the cleanup methods; Hide all ended
 * / Hide ended older than a day call sessionOpen.cleanup, Not now snoozes, and
 * a cleanup offers an ~8s Undo that restores exactly the hidden ids through
 * sessionArchive.setMany. Plus the Paused/Ended labels and the version fence.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, act, renderHook } from '@testing-library/react';
import { SessionCleanupBanner, UNDO_MS, DAY_MS } from '../src/components/SessionCleanupBanner';
import { stoppedCardLabel } from '../src/components/SideBar';
import { parseSessionOpenDoc, stoppedKind, useSessionOpen } from '../src/hooks/useSessionOpen';
import type { SessionOpenDoc } from '../src/types/electron';

const api = window.electronAPI as any;
const KEYS = [
  'sessionOpenCleanup',
  'sessionOpenSnoozeCleanup',
  'sessionArchiveSetMany',
  'sessionOpenGet',
  'onSessionOpenChanged',
] as const;
const saved: Record<string, unknown> = {};

const doc = (cleanup?: Partial<NonNullable<SessionOpenDoc['cleanup']>>): SessionOpenDoc => ({
  version: 3,
  open: { p: 1, e: 2 },
  ended: { e: 3 },
  cleanup: { ended: 25, threshold: 20, suggested: true, snoozedAt: null, ...cleanup },
});

beforeEach(() => {
  for (const k of KEYS) saved[k] = api[k];
  api.sessionOpenCleanup = vi.fn().mockResolvedValue({ hidden: ['a', 'b', 'c'] });
  api.sessionOpenSnoozeCleanup = vi.fn().mockResolvedValue({ ...doc({ suggested: false }), version: 4 });
  api.sessionArchiveSetMany = vi.fn().mockResolvedValue({ version: 9, archived: {} });
});
afterEach(() => {
  for (const k of KEYS) api[k] = saved[k];
  vi.useRealTimers();
});

describe('SessionCleanupBanner', () => {
  it('stays hidden when the hub does not suggest a cleanup, or has no document', () => {
    const { container, rerender } = render(<SessionCleanupBanner doc={null} />);
    expect(container.firstChild).toBeNull();
    rerender(<SessionCleanupBanner doc={doc({ suggested: false })} />);
    expect(container.firstChild).toBeNull();
  });

  it('stays hidden on a hub without the cleanup methods', () => {
    api.sessionOpenCleanup = undefined;
    const { container } = render(<SessionCleanupBanner doc={doc()} />);
    expect(container.firstChild).toBeNull();
  });

  it('hides all ended, then undoes exactly what it hid', async () => {
    render(<SessionCleanupBanner doc={doc()} />);
    expect(screen.getByText('25 ended sessions on your sidebar')).toBeTruthy();
    await act(async () => fireEvent.click(screen.getByText('Hide all ended')));
    expect(api.sessionOpenCleanup).toHaveBeenCalledWith({});
    expect(screen.getByRole('status').textContent).toContain('Hid 3 sessions');
    // The prompt yields to the Undo while it is offered.
    expect(screen.queryByText('Hide all ended')).toBeNull();
    await act(async () => fireEvent.click(screen.getByText('Undo')));
    expect(api.sessionArchiveSetMany).toHaveBeenCalledWith(['a', 'b', 'c'], false);
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('hides only sessions ended more than a day ago', async () => {
    render(<SessionCleanupBanner doc={doc()} />);
    await act(async () => fireEvent.click(screen.getByText('Hide ended older than a day')));
    expect(api.sessionOpenCleanup).toHaveBeenCalledWith({ olderThanMs: DAY_MS });
    expect(DAY_MS).toBe(86_400_000);
  });

  it('Not now snoozes on the hub and adopts the returned document', async () => {
    const onDocument = vi.fn();
    render(<SessionCleanupBanner doc={doc()} onDocument={onDocument} />);
    await act(async () => fireEvent.click(screen.getByText('Not now')));
    expect(api.sessionOpenSnoozeCleanup).toHaveBeenCalledTimes(1);
    expect(onDocument).toHaveBeenCalledWith(expect.objectContaining({ version: 4 }));
    expect(api.sessionOpenCleanup).not.toHaveBeenCalled();
  });

  it('withdraws the Undo after a few seconds', async () => {
    vi.useFakeTimers();
    const view = render(<SessionCleanupBanner doc={doc()} />);
    await act(async () => {
      fireEvent.click(view.getByText('Hide all ended'));
    });
    expect(view.getByRole('status')).toBeTruthy();
    act(() => {
      vi.advanceTimersByTime(UNDO_MS + 1);
    });
    expect(view.queryByRole('status')).toBeNull();
    expect(api.sessionArchiveSetMany).not.toHaveBeenCalled();
  });

  it('says so when a cleanup fails, and offers no Undo', async () => {
    api.sessionOpenCleanup = vi.fn().mockRejectedValue(new Error('hub unavailable'));
    render(<SessionCleanupBanner doc={doc()} />);
    await act(async () => fireEvent.click(screen.getByText('Hide all ended')));
    expect(screen.getByRole('alert').textContent).toContain('hub unavailable');
    expect(screen.queryByText('Undo')).toBeNull();
  });
});

describe('Paused / Ended labels', () => {
  const d = doc();
  it('reads Paused for open ∧ ¬ended, Ended for open ∧ ended, else Stopped', () => {
    expect(stoppedKind(d, 'p')).toBe('paused');
    expect(stoppedKind(d, 'e')).toBe('ended');
    expect(stoppedKind(d, 'x')).toBeNull();
    expect(stoppedKind(null, 'p')).toBeNull();
    expect(stoppedCardLabel(d, 'p', false)).toBe('Paused — click to resume');
    expect(stoppedCardLabel(d, 'e', false)).toBe('Ended — click to respawn');
    expect(stoppedCardLabel(d, 'x', false)).toBe('Stopped — click to respawn');
    expect(stoppedCardLabel(d, 'p', true)).toBe('Paused');
    expect(stoppedCardLabel(undefined, 'p', true)).toBe('Stopped');
  });

  it('parses an early document with no ended map or cleanup block', () => {
    expect(parseSessionOpenDoc({ version: 1, open: { a: 1 } })).toEqual({
      version: 1,
      open: { a: 1 },
      ended: {},
    });
    expect(parseSessionOpenDoc({ version: 'x', open: {} })).toBeNull();
  });
});

describe('useSessionOpen', () => {
  it('reads the document and follows changes, ignoring stale and federated ones', async () => {
    let emit: (doc: unknown) => void = () => {};
    api.sessionOpenGet = vi.fn().mockResolvedValue(doc());
    api.onSessionOpenChanged = vi.fn((cb: (doc: unknown) => void) => {
      emit = cb;
      return () => {};
    });
    const hook = renderHook(() => useSessionOpen());
    await act(async () => {});
    expect(hook.result.current.doc?.version).toBe(3);
    act(() => emit({ ...doc(), version: 2, open: {} }));
    expect(hook.result.current.doc?.version).toBe(3);
    act(() => emit({ ...doc({ suggested: false }), version: 5 }));
    expect(hook.result.current.doc?.cleanup?.suggested).toBe(false);
    act(() => hook.result.current.adopt({ ...doc(), version: 4 }));
    expect(hook.result.current.doc?.version).toBe(5);
    hook.unmount();
  });

  it('stays null on a hub without sessionOpen', async () => {
    api.sessionOpenGet = undefined;
    const hook = renderHook(() => useSessionOpen());
    await act(async () => {});
    expect(hook.result.current.doc).toBeNull();
    hook.unmount();
  });
});
