import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import type { ItemRow } from '../../src/lib/claudemonItems';
import type { Transcript } from '../../src/lib/claudemonSessions';
import ItemDetailOverlay from '../../src/components/ItemDetailOverlay';

const itemsAction = vi.fn();
const sessionsApprove = vi.fn();
const sessionsTranscript = vi.fn();
const sessionsGet = vi.fn();
const onClose = vi.fn();
const onSnoozeMenu = vi.fn();

const itemsClient = {
  list: vi.fn(),
  action: itemsAction,
  subscribe: vi.fn(() => () => {}),
} as any;

const sessionsClient = {
  approve: sessionsApprove,
  getTranscript: sessionsTranscript,
  getSession: sessionsGet,
} as any;

function needsInputItem(overrides: Partial<ItemRow> = {}): ItemRow {
  return {
    id: 'item-99',
    session_id: 'sess-Z',
    state: 'unread',
    priority: 95,
    kind: 'needs_input',
    summary: 'Wants to run prod migration',
    context_paragraph: null,
    next_action: 'approve',
    triggering_event_id: 5,
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

function emptyTranscript(): Transcript {
  return { path: null, messages: [] };
}

describe('ItemDetailOverlay', () => {
  beforeEach(() => {
    itemsAction.mockReset();
    sessionsApprove.mockReset();
    sessionsTranscript.mockReset();
    onClose.mockReset();
    onSnoozeMenu.mockReset();
  });

  it('renders the summary, session name, and priority', () => {
    render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    expect(screen.getByText('Wants to run prod migration')).toBeInTheDocument();
    expect(screen.getByText('feat-auth')).toBeInTheDocument();
    expect(screen.getByText(/priority 95/)).toBeInTheDocument();
  });

  it('closes on Escape', () => {
    const { container } = render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    fireEvent.keyDown(container.querySelector('[aria-label="Item detail"]')!, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('y approves and archives the item', async () => {
    sessionsApprove.mockResolvedValue(undefined);
    itemsAction.mockResolvedValue(needsInputItem({ state: 'resolved' }));
    const { container } = render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    fireEvent.keyDown(container.querySelector('[aria-label="Item detail"]')!, { key: 'y' });
    await waitFor(() => {
      expect(sessionsApprove).toHaveBeenCalledWith('sess-Z', 'yes');
      expect(itemsAction).toHaveBeenCalledWith('item-99', { action: 'archive' });
      expect(onClose).toHaveBeenCalled();
    });
  });

  it('n denies and archives the item', async () => {
    sessionsApprove.mockResolvedValue(undefined);
    itemsAction.mockResolvedValue(needsInputItem({ state: 'resolved' }));
    const { container } = render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    fireEvent.keyDown(container.querySelector('[aria-label="Item detail"]')!, { key: 'n' });
    await waitFor(() => {
      expect(sessionsApprove).toHaveBeenCalledWith('sess-Z', 'no');
      expect(itemsAction).toHaveBeenCalledWith('item-99', { action: 'archive' });
    });
  });

  it('still archives when /approve returns an error (e.g. picker already closed)', async () => {
    sessionsApprove.mockRejectedValue(new Error('409 wrong mode'));
    itemsAction.mockResolvedValue(needsInputItem({ state: 'resolved' }));
    const { container } = render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    fireEvent.keyDown(container.querySelector('[aria-label="Item detail"]')!, { key: 'y' });
    await waitFor(() => {
      expect(itemsAction).toHaveBeenCalledWith('item-99', { action: 'archive' });
      expect(onClose).toHaveBeenCalled();
    });
  });

  it('does not show approve/deny for non-needs_input items', () => {
    const item = needsInputItem({ kind: 'error', priority: 80, next_action: 'review_diff' });
    render(
      <ItemDetailOverlay
        item={item}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    expect(screen.queryByText(/\[y\] approve/)).not.toBeInTheDocument();
    expect(screen.queryByText(/\[n\] deny/)).not.toBeInTheDocument();
  });

  it('s opens the snooze menu via the parent callback and closes the overlay', () => {
    const { container } = render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    fireEvent.keyDown(container.querySelector('[aria-label="Item detail"]')!, { key: 's' });
    expect(onSnoozeMenu).toHaveBeenCalledWith('item-99');
    expect(onClose).toHaveBeenCalled();
  });

  it('Tab cycles through decision → diff → transcript', () => {
    sessionsTranscript.mockResolvedValue(emptyTranscript());
    const { container } = render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    const root = container.querySelector('[aria-label="Item detail"]')!;
    fireEvent.keyDown(root, { key: 'Tab' });
    expect(screen.getByText(/Diff view not implemented yet/)).toBeInTheDocument();
    fireEvent.keyDown(root, { key: 'Tab' });
    expect(screen.getByText(/Loading transcript…|No messages\.|user|assistant/)).toBeInTheDocument();
  });

  it('lazy-loads the transcript only when the transcript tab opens', async () => {
    sessionsTranscript.mockResolvedValue(emptyTranscript());
    const { container } = render(
      <ItemDetailOverlay
        item={needsInputItem()}
        itemsClient={itemsClient}
        sessionsClient={sessionsClient}
        onClose={onClose}
        onSnoozeMenu={onSnoozeMenu}
      />,
    );
    expect(sessionsTranscript).not.toHaveBeenCalled();
    fireEvent.keyDown(container.querySelector('[aria-label="Item detail"]')!, { key: 'Tab' });
    fireEvent.keyDown(container.querySelector('[aria-label="Item detail"]')!, { key: 'Tab' });
    await waitFor(() => expect(sessionsTranscript).toHaveBeenCalledWith('sess-Z'));
  });
});
