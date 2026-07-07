import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import ThemeMaker from '../../src/components/settings/ThemeMaker';
import type { Config } from '../../src/hooks/useConfig';

/**
 * ThemeMaker saves edits to config debounced (~300ms) so the whole app can be
 * the live preview without a save per keystroke. The regression risk is at the
 * edges of that debounce:
 *
 *   - unmounting before the timer fires must FLUSH the pending save (dropping
 *     it would silently revert the user's last edit), and
 *   - a save that already fired naturally must NOT fire again on unmount
 *     (double-saving re-broadcasts config and can clobber later edits).
 */

const themeId = 'custom:my-theme';

function makeConfig(): Config {
  return {
    ui: {
      theme: themeId,
      customThemes: {
        [themeId]: {
          name: 'My Theme',
          base: 'dark',
          colors: {
            accent: 'rgb(12, 34, 56)',
            // Pin the tokens that fall back to `accent`, so exactly one text
            // input displays the accent value below.
            borderActive: '#3355aa',
            busy: '#aa5533',
          },
        },
      },
    },
  } as unknown as Config;
}

/** The free-form text input of the Accent color row. */
const accentInput = () => screen.getByDisplayValue('rgb(12, 34, 56)') as HTMLInputElement;

describe('ThemeMaker debounced save', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('flushes the pending save on unmount so the last edit is not lost', () => {
    const config = makeConfig();
    const save = vi.fn().mockResolvedValue(config);
    const { unmount } = render(<ThemeMaker config={config} save={save} themeId={themeId} />);

    fireEvent.change(accentInput(), { target: { value: '#00ff00' } });
    // Still inside the debounce window — nothing saved yet.
    expect(save).not.toHaveBeenCalled();

    unmount();

    expect(save).toHaveBeenCalledTimes(1);
    const partial = save.mock.calls[0][0] as Partial<Config>;
    expect(partial.ui?.customThemes?.[themeId]?.colors.accent).toBe('#00ff00');

    // The flushed timer must not fire a second save later.
    vi.runAllTimers();
    expect(save).toHaveBeenCalledTimes(1);
  });

  it('does not double-save when the debounce fires naturally before unmount', () => {
    const config = makeConfig();
    const save = vi.fn().mockResolvedValue(config);
    const { unmount } = render(<ThemeMaker config={config} save={save} themeId={themeId} />);

    fireEvent.change(accentInput(), { target: { value: '#00ff00' } });
    vi.advanceTimersByTime(300);
    expect(save).toHaveBeenCalledTimes(1);

    unmount();
    expect(save).toHaveBeenCalledTimes(1);
  });

  it('collapses rapid edits into one save carrying the final value', () => {
    const config = makeConfig();
    const save = vi.fn().mockResolvedValue(config);
    render(<ThemeMaker config={config} save={save} themeId={themeId} />);

    const input = accentInput();
    fireEvent.change(input, { target: { value: '#111111' } });
    fireEvent.change(input, { target: { value: '#222222' } });
    vi.advanceTimersByTime(300);

    expect(save).toHaveBeenCalledTimes(1);
    const partial = save.mock.calls[0][0] as Partial<Config>;
    expect(partial.ui?.customThemes?.[themeId]?.colors.accent).toBe('#222222');
  });
});
