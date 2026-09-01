import React from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ModelContextPopover } from '../../src/components/ModelContextPopover';

afterEach(cleanup);

describe('shared Context popover', () => {
  it('distinguishes requested, effective, provider default, and advertised maximum', () => {
    render(
      <ModelContextPopover
        provider="codex"
        requested={1_000_000}
        effective={258_400}
        providerDefault={272_000}
        advertisedMaximum={872_000}
      />,
    );
    fireEvent.click(screen.getByLabelText('Context settings'));
    expect(screen.getByText(/Effective \(runtime confirmed\): 258k tokens/)).toBeInTheDocument();
    expect(screen.getByText(/Requested: 1.0M tokens/)).toBeInTheDocument();
    expect(screen.getByText(/Provider default: 272k tokens/)).toBeInTheDocument();
    expect(screen.getByText(/Advertised maximum: 872k tokens/)).toBeInTheDocument();
  });

  it('offers numeric Codex selection but leaves unvalidated harnesses provider-managed', () => {
    const change = vi.fn();
    const view = render(
      <ModelContextPopover
        provider="codex"
        requested={1_000_000}
        choices={[{ value: null, label: 'Provider default' }]}
        allowNumeric
        onChange={change}
      />,
    );
    fireEvent.change(screen.getByLabelText('Custom context tokens'), {
      target: { value: '400000' },
    });
    expect(change).toHaveBeenCalledWith(400_000);

    view.rerender(<ModelContextPopover provider="opencode" />);
    fireEvent.click(screen.getByLabelText('Context settings'));
    expect(screen.getByText(/Provider-managed/)).toBeInTheDocument();
    expect(screen.queryByLabelText('Custom context tokens')).toBeNull();
  });
});
