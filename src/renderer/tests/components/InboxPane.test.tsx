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

const sendMessageMock = vi.fn();

vi.mock('../../src/lib/claudemonSessions', () => {
  return {
    ClaudemonSessionsClient: class {
      approve = vi.fn().mockResolvedValue(undefined);
      getTranscript = vi.fn().mockResolvedValue({ path: null, messages: [] });
      getSession = vi.fn().mockResolvedValue(null);
      sendMessage = sendMessageMock;
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
    sendMessageMock.mockReset();
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

  it('opens the detail overlay when Enter is pressed on the selected item', async () => {
    listMock.mockResolvedValue([item()]);
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: 'Enter' });
    expect(await screen.findByRole('region', { hidden: true }).catch(() => null)).toBeNull();
    // Overlay should render its detail-aria-label
    expect(await screen.findByLabelText('Item detail')).toBeInTheDocument();
  });

  it('overlay esc closes back to inbox', async () => {
    listMock.mockResolvedValue([item()]);
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: 'Enter' });
    const overlay = await screen.findByLabelText('Item detail');
    fireEvent.keyDown(overlay, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByLabelText('Item detail')).not.toBeInTheDocument());
  });

  it('o key from L1 spawns a Claude pane attached to the session', async () => {
    listMock.mockResolvedValue([item()]);
    const onAddTab = vi.fn();
    const { container } = render(<InboxPane title="Inbox" isActive onAddTab={onAddTab} />);
    await screen.findByText('feat-auth');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: 'o' });
    expect(onAddTab).toHaveBeenCalledWith(
      'claude',
      undefined,
      'feat-auth',
      undefined,
      undefined,
      undefined,
      'sess-1',
    );
  });

  it('o key in L1 is hidden when no onAddTab is provided', async () => {
    listMock.mockResolvedValue([item()]);
    render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    expect(screen.queryByText(/\[o\] session/)).not.toBeInTheDocument();
  });

  it('groups items into Needs attention and Working sections by priority', async () => {
    listMock.mockResolvedValue([
      item({ id: 'high', priority: 95, summary: 'big deal', session_name: 'feat-auth' }),
      item({ id: 'low', priority: 40, summary: 'small thing', session_name: 'feat-misc', kind: 'done' }),
    ]);
    render(<InboxPane title="Inbox" isActive />);
    expect(await screen.findByText('Needs attention')).toBeInTheDocument();
    expect(await screen.findByText('Working')).toBeInTheDocument();
    expect(await screen.findByText('big deal')).toBeInTheDocument();
    expect(await screen.findByText('small thing')).toBeInTheDocument();
  });

  it('hides snoozed items by default but lists them under a collapsed Snoozed section', async () => {
    listMock.mockResolvedValue([
      item({ id: 'open', priority: 95, summary: 'still active' }),
      item({ id: 'zzz', state: 'snoozed', snoozed_until: 9999, summary: 'sleeping' }),
    ]);
    render(<InboxPane title="Inbox" isActive />);
    expect(await screen.findByText('still active')).toBeInTheDocument();
    expect(await screen.findByText(/Snoozed/)).toBeInTheDocument();
    // Snoozed items hidden under the collapsed section
    expect(screen.queryByText('sleeping')).not.toBeInTheDocument();
    // Click the section header to expand
    fireEvent.click(screen.getByText(/Snoozed/));
    expect(await screen.findByText('sleeping')).toBeInTheDocument();
  });

  it('shows inbox-zero empty state when there are no non-resolved items', async () => {
    listMock.mockResolvedValue([]);
    render(<InboxPane title="Inbox" isActive />);
    expect(await screen.findByText(/Inbox zero/i)).toBeInTheDocument();
  });

  it('opens a search bar on / and filters by session name / summary', async () => {
    listMock.mockResolvedValue([
      item({ id: 'a', summary: 'big deal', session_name: 'feat-auth' }),
      item({ id: 'b', summary: 'small thing', session_name: 'feat-misc' }),
    ]);
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('big deal');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: '/' });
    const input = await screen.findByLabelText('Inbox search');
    fireEvent.change(input, { target: { value: 'misc' } });
    expect(screen.queryByText('big deal')).not.toBeInTheDocument();
    expect(screen.getByText('small thing')).toBeInTheDocument();
    // Esc clears query and closes search
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(await screen.findByText('big deal')).toBeInTheDocument();
    expect(screen.queryByLabelText('Inbox search')).not.toBeInTheDocument();
  });

  it('shows "No matches." when search excludes all items', async () => {
    listMock.mockResolvedValue([item()]);
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    fireEvent.keyDown(container.firstChild as HTMLElement, { key: '/' });
    const input = await screen.findByLabelText('Inbox search');
    fireEvent.change(input, { target: { value: 'nothingmatchesthis' } });
    expect(await screen.findByText(/No matches/)).toBeInTheDocument();
  });

  it('r opens an inline reply, Enter sends the message and snoozes on next_event', async () => {
    listMock.mockResolvedValue([item()]);
    sendMessageMock.mockResolvedValue(undefined);
    actionMock.mockResolvedValue(item({ state: 'snoozed' }));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: 'r' });
    const input = await screen.findByLabelText('Reply input');
    fireEvent.change(input, { target: { value: 'try the new approach instead' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => {
      expect(sendMessageMock).toHaveBeenCalledWith('sess-1', 'try the new approach instead');
      expect(actionMock).toHaveBeenCalledWith('item-1', {
        action: 'snooze_on_event',
        on: 'next_event',
      });
    });
  });

  it('r reply surfaces an error when sendMessage fails (e.g. wrong mode)', async () => {
    listMock.mockResolvedValue([item()]);
    sendMessageMock.mockRejectedValue(new Error('409 wrong mode'));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    fireEvent.keyDown(container.firstChild as HTMLElement, { key: 'r' });
    const input = await screen.findByLabelText('Reply input');
    fireEvent.change(input, { target: { value: 'hello' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(await screen.findByText(/409 wrong mode/)).toBeInTheDocument();
    // Action shouldn't fire when the send failed.
    expect(actionMock).not.toHaveBeenCalled();
  });

  it('Esc closes the reply input without sending', async () => {
    listMock.mockResolvedValue([item()]);
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    fireEvent.keyDown(container.firstChild as HTMLElement, { key: 'r' });
    const input = await screen.findByLabelText('Reply input');
    fireEvent.change(input, { target: { value: 'never mind' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByLabelText('Reply input')).not.toBeInTheDocument());
    expect(sendMessageMock).not.toHaveBeenCalled();
  });

  it('Space toggles multi-select; a archives all selected items', async () => {
    listMock.mockResolvedValue([
      item({ id: 'a', priority: 95, summary: 'first', session_name: 'A' }),
      item({ id: 'b', priority: 90, summary: 'second', session_name: 'B' }),
      item({ id: 'c', priority: 85, summary: 'third', session_name: 'C' }),
    ]);
    actionMock.mockResolvedValue(item({ state: 'resolved' }));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('first');
    const root = container.firstChild as HTMLElement;
    // Select a, j to b, select, j to c, select. Then a (archive all).
    fireEvent.keyDown(root, { key: ' ' });
    fireEvent.keyDown(root, { key: 'j' });
    fireEvent.keyDown(root, { key: ' ' });
    fireEvent.keyDown(root, { key: 'j' });
    fireEvent.keyDown(root, { key: ' ' });
    expect(await screen.findByText(/3 selected/)).toBeInTheDocument();
    fireEvent.keyDown(root, { key: 'a' });
    await waitFor(() => expect(actionMock).toHaveBeenCalledTimes(3));
    const calls = actionMock.mock.calls.map((c) => c[0]).sort();
    expect(calls).toEqual(['a', 'b', 'c']);
  });

  it('Esc clears multi-select without archiving', async () => {
    listMock.mockResolvedValue([
      item({ id: 'a', summary: 'first' }),
      item({ id: 'b', summary: 'second' }),
    ]);
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('first');
    const root = container.firstChild as HTMLElement;
    fireEvent.keyDown(root, { key: ' ' });
    expect(await screen.findByText(/1 selected/)).toBeInTheDocument();
    fireEvent.keyDown(root, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByText(/1 selected/)).not.toBeInTheDocument());
    expect(actionMock).not.toHaveBeenCalled();
  });

  it('a key with no multi-select does not archive the cursor item', async () => {
    listMock.mockResolvedValue([item()]);
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('feat-auth');
    fireEvent.keyDown(container.firstChild as HTMLElement, { key: 'a' });
    expect(actionMock).not.toHaveBeenCalled();
  });

  it('j/k navigation skips items in collapsed sections', async () => {
    listMock.mockResolvedValue([
      item({ id: 'a', priority: 95, summary: 'first', session_name: 'A' }),
      item({ id: 's', state: 'snoozed', snoozed_until: 9999, summary: 'sleeping', session_name: 'S' }),
    ]);
    actionMock.mockResolvedValue(item({ state: 'resolved' }));
    const { container } = render(<InboxPane title="Inbox" isActive />);
    await screen.findByText('first');
    const root = container.firstChild as HTMLElement;
    // Snoozed section collapsed; pressing j shouldn't move into it.
    fireEvent.keyDown(root, { key: 'j' });
    fireEvent.keyDown(root, { key: 'e' });
    // Should have archived "a", not "s".
    await waitFor(() => expect(actionMock).toHaveBeenCalledWith('a', { action: 'archive' }));
  });
});
