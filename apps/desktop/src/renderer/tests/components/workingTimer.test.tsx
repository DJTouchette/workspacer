import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import React from 'react';
import { WorkingTimer } from '../../src/components/claude/WorkingTimer';
import { fmtDuration } from '../../src/components/claude/agentUtils';

/**
 * The "Working for 1m 04s" label beside the streaming spinner. It replaced the
 * inline stop button (which moved into the composer), so this covers the label
 * itself: it ticks on its own interval, it re-bases when a new run starts, and
 * it never renders a negative elapsed time from a start stamped in the future.
 */

afterEach(() => {
  vi.useRealTimers();
});

/** Freeze the clock at `t` (ms) and run on fake timers. */
function freezeAt(t: number) {
  vi.useFakeTimers();
  vi.setSystemTime(t);
}

describe('WorkingTimer', () => {
  it('starts at 0s and ticks once a second', () => {
    freezeAt(10_000);
    render(<WorkingTimer since={10_000} />);
    expect(screen.getByText('Working for 0s')).toBeInTheDocument();

    act(() => void vi.advanceTimersByTime(1000));
    expect(screen.getByText('Working for 1s')).toBeInTheDocument();

    act(() => void vi.advanceTimersByTime(4000));
    expect(screen.getByText('Working for 5s')).toBeInTheDocument();
  });

  it('counts from a start in the past (an attach to a run already going)', () => {
    freezeAt(100_000);
    render(<WorkingTimer since={100_000 - 95_000} />);
    expect(screen.getByText('Working for 1m 35s')).toBeInTheDocument();
  });

  it('re-bases immediately when a new run starts', () => {
    freezeAt(10_000);
    const { rerender } = render(<WorkingTimer since={0} />);
    expect(screen.getByText('Working for 10s')).toBeInTheDocument();
    // New run: the label must not keep showing the old run's elapsed time until
    // the next tick lands.
    act(() => rerender(<WorkingTimer since={10_000} />));
    expect(screen.getByText('Working for 0s')).toBeInTheDocument();
  });

  it('floors at 0 for a start stamped in the future (clock skew)', () => {
    freezeAt(10_000);
    render(<WorkingTimer since={20_000} />);
    expect(screen.getByText('Working for 0s')).toBeInTheDocument();
  });

  it('takes a custom verb', () => {
    freezeAt(5_000);
    render(<WorkingTimer since={2_000} verb="Thinking" />);
    expect(screen.getByText('Thinking for 3s')).toBeInTheDocument();
  });

  it('stops its interval when unmounted', () => {
    freezeAt(0);
    const clear = vi.spyOn(globalThis, 'clearInterval');
    const { unmount } = render(<WorkingTimer since={0} />);
    unmount();
    expect(clear).toHaveBeenCalled();
    clear.mockRestore();
  });
});

describe('fmtDuration — the working label past a minute and an hour', () => {
  it('keeps seconds under a minute', () => {
    expect(fmtDuration(0)).toBe('0s');
    expect(fmtDuration(59_400)).toBe('59s');
  });

  it('switches to m/s at a minute', () => {
    expect(fmtDuration(60_000)).toBe('1m 00s');
    expect(fmtDuration(95_000)).toBe('1m 35s');
    expect(fmtDuration(3_599_000)).toBe('59m 59s');
  });

  it('switches to h/m at an hour — "83m 00s" is not readable', () => {
    expect(fmtDuration(3_600_000)).toBe('1h 00m');
    expect(fmtDuration(3_600_000 + 125_000)).toBe('1h 02m');
    expect(fmtDuration(9_000_000)).toBe('2h 30m');
  });

  it('renders nothing for missing or negative input', () => {
    expect(fmtDuration(undefined)).toBe('');
    expect(fmtDuration(-1)).toBe('');
  });
});
