/**
 * Injected fleet/supervisor wakes render as structured cards, not paragraph
 * bubbles: ConversationMessage recognizes the shared wire format (built and
 * parsed by main/shared/fleetMessages.ts) on user turns, shows a kind badge +
 * per-worker rows with a session chip and a collapsed last-reply, and leaves
 * every other user message on the raw-bubble path.
 */
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ConversationMessage } from '../src/components/claude/ConversationMessage';
import { buildFleetMessage } from '../../main/shared/fleetMessages';
import type { ConversationTurn } from '../src/types/claudeSession';

const turn = (content: string): ConversationTurn => ({ role: 'user', content });

describe('<ConversationMessage> fleet wake cards', () => {
  it('renders a worker-finished wake as a card with a collapsed last reply', () => {
    const text = buildFleetMessage('worker-finished', [
      {
        label: 'alpha: fix tests',
        sessionId: 'w1',
        cwd: '/home/u/Work/alpha',
        lastReply: 'All 42 tests pass. Done.',
      },
    ]);
    render(<ConversationMessage turn={turn(text)} />);

    expect(screen.getByText(/worker finished/i)).toBeTruthy();
    expect(screen.getByText('alpha: fix tests')).toBeTruthy();
    expect(screen.getByText('session:w1')).toBeTruthy();
    expect(screen.getByTitle('/home/u/Work/alpha')).toBeTruthy();
    // The instruction tail is manager-facing noise — the card drops it.
    expect(screen.queryByText(/brief\.md/)).toBeNull();

    // Reply excerpt starts collapsed, expands on the toggle.
    expect(screen.queryByText('All 42 tests pass. Done.')).toBeNull();
    fireEvent.click(screen.getByText('last reply'));
    expect(screen.getByText('All 42 tests pass. Done.')).toBeTruthy();
  });

  it('marks a stopped/killed worker with a chip, and hides the full-message block behind the card', () => {
    const text = buildFleetMessage('worker-finished', [
      {
        label: 'alpha: fix tests',
        sessionId: 'w1',
        cwd: '/home/u/Work/alpha',
        stopped: true,
        lastReply: 'was mid-refactor',
        fullReply: 'was mid-refactor\nwhen the session was killed',
      },
    ]);
    render(<ConversationMessage turn={turn(text)} />);
    expect(screen.getByText('stopped/killed')).toBeTruthy();
    // The agent-facing full-message block never leaks into the card.
    expect(screen.queryByText(/Full final message/)).toBeNull();
  });

  it('renders a supervisor block wake with its blocked-on chip', () => {
    const text = buildFleetMessage('blocked', [
      { label: 'beta: ship', sessionId: 'w2', blockedOn: 'question' },
    ]);
    render(<ConversationMessage turn={turn(text)} />);
    expect(screen.getByText(/agent blocked/i)).toBeTruthy();
    expect(screen.getByText('has a question')).toBeTruthy();
    expect(screen.getByText('session:w2')).toBeTruthy();
  });

  it('renders a worker progress update as its own face — never a completion', () => {
    const text = buildFleetMessage('progress', [
      {
        label: 'rust: stream approvals',
        sessionId: 'w9',
        cwd: '/home/u/Work/wks',
        note: 'Read most of claude_stream.rs; I have written no code yet.',
        needsDecision: true,
      },
    ]);
    render(<ConversationMessage turn={turn(text)} />);

    expect(screen.getByText(/progress update/i)).toBeTruthy();
    // The one thing this card must never look like.
    expect(screen.queryByText(/worker finished/i)).toBeNull();
    expect(screen.getByText('needs a decision')).toBeTruthy();
    // The note is shown OPEN — no toggle to click.
    expect(
      screen.getByText('Read most of claude_stream.rs; I have written no code yet.'),
    ).toBeTruthy();
    expect(screen.queryByText('last reply')).toBeNull();
  });

  it('leaves ordinary user text on the bubble path', () => {
    render(<ConversationMessage turn={turn('Worker finished the [fleet] job, nice')} />);
    expect(screen.getByText('Worker finished the [fleet] job, nice')).toBeTruthy();
    expect(screen.queryByText(/session:/)).toBeNull();
  });

  it('a Reply click on an entry hands the parent that entry, not another one in a multi-entry wake', () => {
    const text = buildFleetMessage('worker-finished', [
      { label: 'alpha: fix tests', sessionId: 'w1', cwd: '/home/u/Work/alpha' },
      { label: 'beta: ship', sessionId: 'w2', cwd: '/home/u/Work/beta' },
    ]);
    const onReply = vi.fn();
    render(<ConversationMessage turn={turn(text)} onReply={onReply} />);

    fireEvent.click(screen.getByTitle(/Reply to beta: ship/));
    expect(onReply).toHaveBeenCalledTimes(1);
    expect(onReply).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: 'w2', label: 'beta: ship' }),
    );
  });

  it('renders no Reply affordance when the parent does not wire onReply', () => {
    const text = buildFleetMessage('worker-finished', [
      { label: 'alpha: fix tests', sessionId: 'w1', cwd: '/home/u/Work/alpha' },
    ]);
    render(<ConversationMessage turn={turn(text)} />);
    expect(screen.queryByTitle(/Reply to/)).toBeNull();
  });
});
