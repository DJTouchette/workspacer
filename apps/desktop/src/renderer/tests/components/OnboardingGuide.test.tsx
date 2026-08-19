import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import Onboarding from '../../src/components/Onboarding';
import { GUIDE_PRESETS } from '../../src/lib/guide';

/**
 * The welcome card's "Or just ask" tour section: present only when onAskGuide
 * is wired, chips fire it with the full preset prompt, and the usage fine
 * print sits right beside the chips (the click IS the informed consent).
 */

const baseProps = {
  onSpawn: () => {},
  onDismiss: () => {},
  shortcuts: {},
};

describe('Onboarding guide section', () => {
  it('renders the tour chips, teaser bubble, and usage note when wired', () => {
    render(<Onboarding {...baseProps} onAskGuide={() => {}} />);

    expect(screen.getByText(/built-in guide/)).toBeInTheDocument();
    expect(screen.getByText(/uses a little of your usage/)).toBeInTheDocument();
    for (const preset of GUIDE_PRESETS.slice(0, 3)) {
      expect(screen.getByRole('button', { name: preset.label })).toBeInTheDocument();
    }
  });

  it('fires onAskGuide with the full preset prompt on chip click', () => {
    const onAskGuide = vi.fn();
    render(<Onboarding {...baseProps} onAskGuide={onAskGuide} />);

    fireEvent.click(screen.getByRole('button', { name: GUIDE_PRESETS[0].label }));
    expect(onAskGuide).toHaveBeenCalledWith(GUIDE_PRESETS[0].prompt);
  });

  it('hides the section entirely when onAskGuide is absent', () => {
    render(<Onboarding {...baseProps} />);
    expect(screen.queryByText(/built-in guide/)).not.toBeInTheDocument();
  });
});
