/**
 * Fresh-install peek/gap fallbacks. The declared config-schema defaults are 0
 * (configService.ts DEFAULTS.panes), but the renderer keeps inline `??`
 * fallbacks for configs that predate the keys — and this exact split already
 * shipped once: the defaults moved to 0 while stale `?? 80` / `?? 16` literals
 * kept every fresh install (config file without a `panes` key) on the old
 * 80px peek / 16px gap. These tests pin the runtime fallbacks to 0 so a future
 * default change (or a reverted merge) can't silently reintroduce the split.
 */

import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import React from 'react';
import LayoutSection from '../../src/components/settings/LayoutSection';
import type { Config } from '../../src/hooks/useConfig';

function renderSection(config: Partial<Config>) {
  render(<LayoutSection config={config as Config} save={vi.fn()} />);
}

describe('LayoutSection — peek/gap slider fallbacks on a fresh install', () => {
  it('keeps intent workspaces off by default and saves the setting independently of focus mode', async () => {
    const save = vi.fn().mockResolvedValue({});
    render(<LayoutSection config={{ ui: { mode: 'focus' } } as Config} save={save} />);
    const toggle = screen.getByRole('checkbox', { name: 'Intent workspaces (preview)' });
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);
    expect(save).toHaveBeenCalledWith({ ui: { mode: 'focus', intentWorkspaces: true } });
  });

  it('shows a failed setting save', async () => {
    render(
      <LayoutSection
        config={{} as Config}
        save={vi.fn().mockRejectedValue(new Error('Config write refused'))}
      />,
    );
    fireEvent.click(screen.getByRole('checkbox', { name: 'Intent workspaces (preview)' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Config write refused');
    expect(screen.getByRole('checkbox')).not.toBeChecked();
  });

  it('renders both sliders at 0 with 0px labels when config.panes is absent', () => {
    renderSection({}); // fresh install: no `panes` key in the config file

    const sliders = screen.getAllByRole('slider') as HTMLInputElement[];
    expect(sliders).toHaveLength(2); // peek + gap
    for (const slider of sliders) expect(slider.value).toBe('0');
    // Labels must agree with the slider values (peek '0px' and gap '0px'),
    // matching the declared default (configService DEFAULTS: peek 0, gap 0).
    expect(screen.getAllByText('0px')).toHaveLength(2);
  });

  it('still renders configured values verbatim (no fallback interference)', () => {
    renderSection({ panes: { peek: 80, gap: 16, insertPosition: 'after' } } as Partial<Config>);

    const [peek, gap] = screen.getAllByRole('slider') as HTMLInputElement[];
    expect(peek.value).toBe('80');
    expect(gap.value).toBe('16');
    expect(screen.getByText('80px')).toBeInTheDocument();
    expect(screen.getByText('16px')).toBeInTheDocument();
  });
});
