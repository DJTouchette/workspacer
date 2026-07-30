import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import React from 'react';
import { ConversationMessage } from '../../src/components/claude/ConversationMessage';

/**
 * The hover-revealed copy affordance on agent messages. The reveal itself is
 * CSS (jsdom applies no stylesheet), so what's worth pinning down is the part
 * that can actually break: the button exists on agent text only, it copies the
 * raw markdown rather than the rendered text, and it doesn't leave a lying
 * "Copied" tick behind when the clipboard refuses.
 */

const writeText = vi.fn().mockResolvedValue(undefined);

beforeEach(() => {
  writeText.mockClear();
  writeText.mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText },
    configurable: true,
  });
});

describe('agent message copy button', () => {
  it('copies the raw markdown of an assistant message', async () => {
    const content = 'Fixed it in **`src/app.ts`** — see the diff.';
    render(<ConversationMessage turn={{ role: 'assistant', content, timestamp: 1 }} />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(content));
  });

  it('confirms with a Copied state, then goes back to Copy', async () => {
    vi.useFakeTimers();
    try {
      render(<ConversationMessage turn={{ role: 'assistant', content: 'done', timestamp: 1 }} />);
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
      // Flush the clipboard promise.
      await act(async () => {
        await Promise.resolve();
      });
      expect(screen.getByRole('button', { name: 'Copied' })).toBeInTheDocument();
      act(() => void vi.advanceTimersByTime(1500));
      expect(screen.getByRole('button', { name: 'Copy message' })).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('stays in the un-copied state when the clipboard is denied', async () => {
    writeText.mockRejectedValue(new Error('denied'));
    render(<ConversationMessage turn={{ role: 'assistant', content: 'done', timestamp: 1 }} />);
    fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByRole('button', { name: 'Copied' })).toBeNull();
  });

  it('is not offered on the user’s own bubble', () => {
    render(<ConversationMessage turn={{ role: 'user', content: 'do the thing', timestamp: 1 }} />);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('renders no action row for a text-less assistant turn', () => {
    const { container } = render(
      <ConversationMessage turn={{ role: 'assistant', content: '', timestamp: 1 }} />,
    );
    expect(container.querySelector('.wks-hover-actions')).toBeNull();
  });
});
