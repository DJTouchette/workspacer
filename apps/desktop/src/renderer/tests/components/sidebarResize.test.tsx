import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import React from 'react';
import { SidebarResizeHandle } from '../../src/components/SidebarResizeHandle';
import { SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MIN_WIDTH } from '../../src/lib/sidebarWidth';

/**
 * The sidebar's drag handle. The contract that matters is the split between the
 * two callbacks: `onResize` may fire all through a gesture (it only moves
 * layout), `onCommit` is the one that writes config.yaml and must fire once, on
 * settle, and never for a gesture that changed nothing.
 */

const handle = () => screen.getByRole('separator', { name: 'Resize sidebar' });

function setup(width = SIDEBAR_DEFAULT_WIDTH) {
  const onResize = vi.fn();
  const onCommit = vi.fn();
  const view = render(
    <SidebarResizeHandle width={width} onResize={onResize} onCommit={onCommit} />,
  );
  return { onResize, onCommit, view };
}

/** Drag from `from` to `to` (clientX), leaving the button released. */
function drag(el: Element, from: number, to: number) {
  fireEvent.pointerDown(el, { button: 0, pointerId: 1, clientX: from });
  fireEvent.pointerMove(el, { pointerId: 1, clientX: to });
  fireEvent.pointerUp(el, { pointerId: 1, clientX: to });
}

beforeEach(() => {
  // jsdom's default window is 1024 wide; the viewport share cap is derived from
  // it, so keep it explicit rather than inherited.
  Object.defineProperty(window, 'innerWidth', { value: 1600, configurable: true });
});

afterEach(() => {
  document.body.style.userSelect = '';
  document.body.style.cursor = '';
});

describe('SidebarResizeHandle', () => {
  it('reports the dragged width and commits it once on release', async () => {
    const { onResize, onCommit } = setup(300);
    drag(handle(), 300, 380);
    await waitFor(() => expect(onResize).toHaveBeenCalledWith(380));
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith(380);
  });

  it('clamps a drag past the minimum instead of collapsing the panel', async () => {
    const { onCommit } = setup(300);
    drag(handle(), 300, 10);
    await waitFor(() => expect(onCommit).toHaveBeenCalledWith(SIDEBAR_MIN_WIDTH));
  });

  it('clamps a drag past the viewport share cap', async () => {
    Object.defineProperty(window, 'innerWidth', { value: 900, configurable: true });
    const { onCommit } = setup(300);
    drag(handle(), 300, 900);
    // 45% of a 900px window.
    await waitFor(() => expect(onCommit).toHaveBeenCalledWith(405));
  });

  it('does not write config for a click that moved nothing', () => {
    const { onCommit } = setup(300);
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1, clientX: 300 });
    fireEvent.pointerUp(handle(), { pointerId: 1, clientX: 300 });
    expect(onCommit).not.toHaveBeenCalled();
  });

  it('ignores a non-primary button (that belongs to the context menu)', () => {
    const { onResize, onCommit } = setup(300);
    fireEvent.pointerDown(handle(), { button: 2, pointerId: 1, clientX: 300 });
    fireEvent.pointerMove(handle(), { pointerId: 1, clientX: 400 });
    fireEvent.pointerUp(handle(), { pointerId: 1, clientX: 400 });
    expect(onResize).not.toHaveBeenCalled();
    expect(onCommit).not.toHaveBeenCalled();
  });

  it('restores text selection and the cursor after a drag', async () => {
    setup(300);
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1, clientX: 300 });
    expect(document.body.style.userSelect).toBe('none');
    expect(document.body.style.cursor).toBe('col-resize');
    fireEvent.pointerUp(handle(), { pointerId: 1, clientX: 320 });
    expect(document.body.style.userSelect).toBe('');
    expect(document.body.style.cursor).toBe('');
  });

  it('double-click resets to the shipped default', () => {
    const { onResize, onCommit } = setup(420);
    fireEvent.doubleClick(handle());
    expect(onResize).toHaveBeenCalledWith(SIDEBAR_DEFAULT_WIDTH);
    expect(onCommit).toHaveBeenCalledWith(SIDEBAR_DEFAULT_WIDTH);
  });

  it('arrow keys nudge live and coalesce into a single commit', () => {
    vi.useFakeTimers();
    try {
      const { onResize, onCommit } = setup(300);
      fireEvent.keyDown(handle(), { key: 'ArrowRight' });
      fireEvent.keyDown(handle(), { key: 'ArrowRight' });
      // Both nudges read the same `width` prop (the parent hasn't re-rendered in
      // this test), so what matters is that neither wrote config yet.
      expect(onResize).toHaveBeenCalledTimes(2);
      expect(onCommit).not.toHaveBeenCalled();
      act(() => void vi.advanceTimersByTime(500));
      expect(onCommit).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('Shift+Arrow takes a bigger step than a bare arrow', () => {
    const { onResize } = setup(300);
    fireEvent.keyDown(handle(), { key: 'ArrowRight' });
    fireEvent.keyDown(handle(), { key: 'ArrowRight', shiftKey: true });
    const [[small], [large]] = onResize.mock.calls;
    expect(large - 300).toBeGreaterThan(small - 300);
  });
});
