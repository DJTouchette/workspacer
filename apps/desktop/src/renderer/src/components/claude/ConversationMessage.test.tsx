import { describe, it, expect } from 'vitest';
import '@testing-library/jest-dom/vitest';
import { render, screen } from '@testing-library/react';
import { ConversationMessage } from './ConversationMessage';
import type { ConversationTurn } from '../../types/claudeSession';

function turn(content: string, role: ConversationTurn['role'] = 'assistant'): ConversationTurn {
  return { role, content, timestamp: Date.now() };
}

describe('ConversationMessage — credit-balance remedy', () => {
  it('attaches the remedy under the raw bubble for the marker-prefixed error text', () => {
    render(<ConversationMessage turn={turn('⚠️ Error: Credit balance is too low.')} />);
    expect(screen.getAllByText(/Credit balance is too low/).length).toBeGreaterThan(1);
    expect(screen.getByText(/stale or wrong credentials/)).toBeInTheDocument();
    expect(screen.getByText('claude')).toBeInTheDocument();
  });

  it('matches the doubled/concatenated render observed live', () => {
    render(
      <ConversationMessage
        turn={turn('Credit balance is too lowCredit balance is too low')}
      />,
    );
    expect(screen.getByText(/stale or wrong credentials/)).toBeInTheDocument();
  });

  it('does not attach the remedy to an unrelated 529 overload', () => {
    render(
      <ConversationMessage
        turn={turn('⚠️ Error: API Error: 529 Overloaded. This is a server-side issue, usually temporary')}
      />,
    );
    expect(screen.queryByText(/stale or wrong credentials/)).not.toBeInTheDocument();
  });

  it('never attaches the remedy to a user turn, even if it quotes the error', () => {
    render(<ConversationMessage turn={turn('Credit balance is too low', 'user')} />);
    expect(screen.queryByText(/stale or wrong credentials/)).not.toBeInTheDocument();
  });
});
