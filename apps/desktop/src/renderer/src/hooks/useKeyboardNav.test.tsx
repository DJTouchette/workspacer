/**
 * Chord lifetime.
 *
 * The key handler is registered by one big effect whose dependency list is ~30
 * callbacks from App. Several of those change identity on every session
 * snapshot, i.e. many times a second while any agent is streaming, so that
 * effect re-subscribes constantly. React runs an effect's cleanup before it
 * re-runs — so anything the cleanup tears down is torn down constantly.
 *
 * That is how a half-typed chord came to be cancelled the instant the fleet got
 * busy: leader chords worked perfectly on an idle app and died under load,
 * which reads as flakiness rather than as a bug. These tests pin the rule that
 * a chord survives a re-subscribe and is reset only on unmount or a rebind.
 */
import { describe, expect, it, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import { useKeyboardNav } from './useKeyboardNav';

const noop = () => {};

function Harness({
  onChordPathChange,
  onNextAttention,
  prefix = 'ctrl+space',
  addTab = () => 'new',
}: {
  onChordPathChange: (path: string[] | null) => void;
  onNextAttention: () => void;
  prefix?: string;
  addTab?: (type: string) => string;
}) {
  useKeyboardNav({
    tabs: [],
    activeTabId: 't1',
    setActiveTabId: noop,
    scrollToTab: noop,
    addTab: addTab as never,
    splitTab: () => 'new',
    removeTab: noop,
    removePane: noop,
    renameTab: noop,
    moveTab: noop,
    setActivePane: noop,
    onToggleHelp: noop,
    prefix,
    onChordPathChange,
    onNextAttention,
    shortcuts: { 'new-terminal': 'prefix t' },
  });
  return null;
}

/** Press the leader, which arms the chord at its root path. */
const pressLeader = (): void => {
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: ' ', code: 'Space', ctrlKey: true, bubbles: true }),
    );
  });
};

describe('useKeyboardNav chord lifetime', () => {
  it('arms the chord when the leader is pressed', () => {
    const onChordPathChange = vi.fn();
    render(<Harness onChordPathChange={onChordPathChange} onNextAttention={noop} />);
    pressLeader();
    expect(onChordPathChange).toHaveBeenLastCalledWith([]);
  });

  it('keeps a half-typed chord alive when the key-handler effect re-subscribes', () => {
    const onChordPathChange = vi.fn();
    const { rerender } = render(
      <Harness onChordPathChange={onChordPathChange} onNextAttention={() => {}} />,
    );
    pressLeader();
    onChordPathChange.mockClear();

    // A new onNextAttention identity — exactly what a session snapshot landing
    // produced ~60 times a second — re-runs the effect, cleanup first.
    rerender(<Harness onChordPathChange={onChordPathChange} onNextAttention={() => {}} />);
    rerender(<Harness onChordPathChange={onChordPathChange} onNextAttention={() => {}} />);

    expect(onChordPathChange).not.toHaveBeenCalledWith(null);
  });

  it('still resolves a chord after a re-subscribe', () => {
    const addTab = vi.fn(() => 'new');
    const onChordPathChange = vi.fn();
    const { rerender } = render(
      <Harness onChordPathChange={onChordPathChange} onNextAttention={() => {}} addTab={addTab} />,
    );
    pressLeader();
    rerender(
      <Harness onChordPathChange={onChordPathChange} onNextAttention={() => {}} addTab={addTab} />,
    );

    // 't' completes `prefix t` (new-terminal) — only reachable if the chord
    // survived the re-subscribe. Before the fix this keystroke fell through to
    // the direct-binding loop and did nothing.
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 't', bubbles: true }));
    });
    expect(addTab).toHaveBeenCalledWith('terminal');
  });

  it('does not resolve a chord that was never armed', () => {
    const addTab = vi.fn(() => 'new');
    render(<Harness onChordPathChange={vi.fn()} onNextAttention={noop} addTab={addTab} />);
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 't', bubbles: true }));
    });
    expect(addTab).not.toHaveBeenCalled();
  });

  it('cancels a half-typed chord when the leader itself is rebound', () => {
    const onChordPathChange = vi.fn();
    const { rerender } = render(
      <Harness onChordPathChange={onChordPathChange} onNextAttention={noop} />,
    );
    pressLeader();
    onChordPathChange.mockClear();

    rerender(
      <Harness onChordPathChange={onChordPathChange} onNextAttention={noop} prefix="alt+space" />,
    );
    expect(onChordPathChange).toHaveBeenCalledWith(null);
  });

  it('cancels a half-typed chord on unmount', () => {
    const onChordPathChange = vi.fn();
    const { unmount } = render(
      <Harness onChordPathChange={onChordPathChange} onNextAttention={noop} />,
    );
    pressLeader();
    onChordPathChange.mockClear();

    unmount();
    expect(onChordPathChange).toHaveBeenCalledWith(null);
  });

  it('stops listening after unmount', () => {
    const onChordPathChange = vi.fn();
    const { unmount } = render(
      <Harness onChordPathChange={onChordPathChange} onNextAttention={noop} />,
    );
    unmount();
    onChordPathChange.mockClear();
    pressLeader();
    expect(onChordPathChange).not.toHaveBeenCalled();
  });
});
