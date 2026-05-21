import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import type { ItemRow, ItemChange } from '../../src/lib/claudemonItems';

// Mock the items client before importing InboxPane.
const listMock = vi.fn();
const actionMock = vi.fn();
let subscribeCb: ((change: ItemChange) => void) | null = null;
const subscribeMock = vi.fn((onChange: (c: ItemChange) => void) => {
  subscribeCb = onChange;
  return () => {
    subscribeCb = null;
  };
});

vi.mock('../../src/lib/claudemonItems', () => {
  return {
    ClaudemonItemsClient: class {
      list = listMock;
      action = actionMock;
      subscribe = subscribeMock;
    },
  };
});

import InboxPane from '../../src/panes/InboxPane';

function item(overrides: Partial<ItemRow> = {}): ItemRow {
  return {
    id: 'item-1',
    session_id: 'sess-1',
    state: 'unread',
    priority: 95,
    kind: 'needs_input',
    summary: 'Needs decision on Bash call',
    context_paragraph: null,
    next_action: 'approve',
    triggering_event_id: 2,
    created_at: 1000,
    updated_at: 1000,
    resolved_at: null,
    snoozed_until: null,
    snoozed_on_event: null,
    flagged: false,
    session_name: 'feat-auth',
    session_project: 'feat-auth',
    session_state: 'needs_input',
    ...overrides,
  };
}

describe('InboxPane', () => {
  beforeEach(() => {
    listMock.mockReset();
    actionMock.mockReset();
    subscribeMock.mockClear();
    subscribeCb = null;
  });

  it('renders empty state when no items are returned', async () => {
    listMock.mockResolvedValue([]);
    render(<InboxPane title="Inbox" isActive />);
    await waitFor(() => expect(listMock).toHaveBeenCalled());
    expect(await screen.findByText(/Inbox zero/i)).toBeInTheDocument();
  });

  it('renders fetched items with their summary and session name', async () => {
    listMock.mockResolvedValue([item()]);
    render(<InboxPane title="Inbox" isActive />);
    expect(await screen.findByText('feat-auth')).toBeInTheDocument();
    expect(await screen.findByText('Needs decision on Bash call')).toBeInTheDocument();
  });

  it('appends new items arriving via the SSE stream', async () => {
    listMock.mockResolvedValue([]);
    render(<InboxPane title="Inbox" isActive />);
    await waitFor(() => expect(subscribeCb).toBeTruthy());
    subscribeCb!({ type: 'item_created', item: item({ id: 'live-1', summary: 'Tool Edit failed' }) });
    expect(await screen.findByText('Tool Edit failed')).toBeInTheDocument();
  });

  it('archives the selected item when e is pressed', async () => {
    listMock.mockResolvedValue([item()]);
    actionMock.mockResolvedValue(item({ state: 'resolved' }));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: 'e' });
    await waitFor(() =>
      expect(actionMock).toHaveBeenCalledWith('item-1', { action: 'archive' }),
    );
  });

  it('flags and unflags the selected item with !', async () => {
    listMock.mockResolvedValue([item()]);
    actionMock.mockResolvedValue(item({ flagged: true }));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: '!' });
    await waitFor(() => expect(actionMock).toHaveBeenCalledWith('item-1', { action: 'flag' }));
  });

  it('opens a snooze menu on s and applies the chosen duration', async () => {
    listMock.mockResolvedValue([item()]);
    actionMock.mockResolvedValue(item({ state: 'snoozed' }));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: 's' });
    // Menu hint should be visible
    expect(await screen.findByText(/\[1\] 15m/)).toBeInTheDocument();
    fireEvent.keyDown(root, { key: '1' });
    await waitFor(() => {
      expect(actionMock).toHaveBeenCalledTimes(1);
      const [, action] = actionMock.mock.calls[0];
      expect(action.action).toBe('snooze_until');
      expect(typeof action.until).toBe('number');
    });
  });

  it('removes resolved items when an item_resolved event arrives', async () => {
    listMock.mockResolvedValue([item()]);
    render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    subscribeCb!({ type: 'item_resolved', id: 'item-1', session_id: 'sess-1' });
    await waitFor(() => expect(screen.queryByText('feat-auth')).not.toBeInTheDocument());
    expect(await screen.findByText(/Inbox zero/)).toBeInTheDocument();
  });

  it('navigates the selection with j/k', async () => {
    listMock.mockResolvedValue([
      item({ id: 'a', summary: 'first', priority: 95, session_name: 'A' }),
      item({ id: 'b', summary: 'second', priority: 80, session_name: 'B' }),
    ]);
    actionMock.mockResolvedValue(item({ state: 'resolved' }));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('first');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: 'j' });
    fireEvent.keyDown(root, { key: 'e' });
    await waitFor(() =>
      expect(actionMock).toHaveBeenCalledWith('b', { action: 'archive' }),
    );
  });

  it('surfaces an error status when the initial list call fails', async () => {
    listMock.mockRejectedValue(new Error('boom'));
    render(<InboxPane title="Inbox" isActive />);
    expect(await screen.findByText(/boom/)).toBeInTheDocument();
  });
});
