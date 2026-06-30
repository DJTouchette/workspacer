/**
 * Regression test: keyboard navigation order must match the visual row order.
 *
 * The palette renders groups as actions → apps → commands → … but the unified
 * item array (which ArrowUp/Down step through, and which each row highlights by
 * its index in) had commands BEFORE apps. So when both an app and a command
 * were present, ArrowDown jumped past the app to the last row and then back up —
 * the highlight bounced instead of advancing one visual row at a time.
 */
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import CommandPalette from '../src/components/CommandPalette';

function paletteRows(container: HTMLElement): HTMLElement[] {
  return (Array.from(container.querySelectorAll('div')) as HTMLElement[]).filter(
    (el) => el.style.cursor === 'pointer' && el.style.gap === '10px',
  );
}

const highlightedIndex = (rows: HTMLElement[]) =>
  rows.findIndex((r) => r.style.backgroundColor === 'var(--wks-bg-selected)');

describe('CommandPalette — keyboard nav order matches visual order', () => {
  it('ArrowDown advances the highlight one visual row at a time', () => {
    const { container } = render(
      <CommandPalette
        visible
        apps={[{ name: 'MyApp', url: 'https://example.com' }]}
        onClose={vi.fn()}
        onLaunchApp={vi.fn()}
        onAddTab={vi.fn()}
        onSpawnAgent={vi.fn()} // surfaces a "Spawn Agent" command row
      />,
    );

    const input = container.querySelector('input')!;
    const rowCount = paletteRows(container).length;
    // Sanity: both an app and at least one command row are present (the
    // visual-vs-nav-order bug needs a mixed list). Kept as a robust lower bound
    // rather than an exact count so it doesn't break when commands are added/removed.
    expect(rowCount).toBeGreaterThanOrEqual(2);

    // selectedIndex starts at 0; after k ArrowDowns it is k. The highlighted
    // row's position in the DOM must equal k for every step — i.e. the visual
    // order and the navigation order are the same list.
    for (let k = 0; k < rowCount; k++) {
      const rows = paletteRows(container);
      expect(highlightedIndex(rows)).toBe(k);
      fireEvent.keyDown(input, { key: 'ArrowDown' });
    }
  });
});
