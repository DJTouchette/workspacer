import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import HomeSpace from '../../src/components/HomeSpace';

/**
 * The "nothing selected" home screen: the spawn CTA must exist and fire
 * onSpawn, and the spawn shortcut renders split into per-key kbd chips
 * (not as one opaque "ctrl+shift+n" string).
 */

describe('HomeSpace', () => {
  it('renders the spawn CTA and fires onSpawn on click', () => {
    const onSpawn = vi.fn();
    render(<HomeSpace onSpawn={onSpawn} />);

    const cta = screen.getByRole('button', { name: /Spawn agent/ });
    fireEvent.click(cta);
    expect(onSpawn).toHaveBeenCalledTimes(1);
  });

  it('splits the spawn shortcut into one kbd chip per key', () => {
    render(<HomeSpace onSpawn={() => {}} spawnShortcut="ctrl+shift+n" />);

    // Each key is its own chip element…
    const ctrl = screen.getByText('ctrl');
    const shift = screen.getByText('shift');
    const n = screen.getByText('n');
    expect(ctrl).not.toBe(shift);
    expect(shift).not.toBe(n);
    // …and no chip carries the raw joined form.
    expect(screen.queryByText('ctrl+shift+n')).not.toBeInTheDocument();
  });

  it('renders no shortcut hint when none is configured', () => {
    render(<HomeSpace onSpawn={() => {}} />);
    expect(screen.queryByText('ctrl')).not.toBeInTheDocument();
    // The CTA is still there regardless.
    expect(screen.getByRole('button', { name: /Spawn agent/ })).toBeInTheDocument();
  });
});
