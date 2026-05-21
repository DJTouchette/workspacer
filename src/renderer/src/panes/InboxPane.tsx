/**
 * L1 inbox view (spec §5). Subscribes to claudemon's /items/stream and
 * renders one row per item, with j/k navigation and single-key actions for
 * archive / flag / snooze. L2 / L3 drill-down is wired as a stub for later
 * phases.
 */

import React, { useCallback, useEffect, useMemo, useReducer, useRef } from 'react';
import ItemDetailOverlay from '../components/ItemDetailOverlay';
import {
  ClaudemonItemsClient,
  ItemAction,
  ItemChange,
  ItemRow,
} from '../lib/claudemonItems';
import { ClaudemonSessionsClient } from '../lib/claudemonSessions';

interface InboxPaneProps {
  title: string;
  isActive: boolean;
  /**
   * Optional bridge to spawn new tabs/panes. When provided, the `o` keystroke
   * on a selected item opens a Claude pane attached to that session.
   * Signature mirrors App.tsx's addTab.
   */
  onAddTab?: (
    type: 'claude',
    shell?: string,
    label?: string,
    cwd?: string,
    profileId?: string,
    resumeSessionId?: string,
    attachSessionId?: string,
  ) => void;
}

type Status = 'connecting' | 'connected' | 'error';

interface State {
  items: Record<string, ItemRow>;
  order: string[];
  selectedId: string | null;
  status: Status;
  snoozeMenuFor: string | null;
  detailFor: string | null;
  lastError: string | null;
}

type Action =
  | { type: 'hydrate'; items: ItemRow[] }
  | { type: 'apply_change'; change: ItemChange }
  | { type: 'set_status'; status: Status; error?: string | null }
  | { type: 'select'; delta: number }
  | { type: 'select_id'; id: string | null }
  | { type: 'open_snooze_menu'; id: string }
  | { type: 'close_snooze_menu' }
  | { type: 'open_detail'; id: string }
  | { type: 'close_detail' };

function sortItems(items: ItemRow[]): string[] {
  return [...items]
    .sort((a, b) => {
      if (a.flagged !== b.flagged) return a.flagged ? -1 : 1;
      if (a.priority !== b.priority) return b.priority - a.priority;
      return b.updated_at - a.updated_at;
    })
    .map((i) => i.id);
}

function visibleItems(state: State): ItemRow[] {
  return state.order
    .map((id) => state.items[id])
    .filter((it): it is ItemRow => it !== undefined && it.state !== 'resolved' && it.state !== 'snoozed');
}

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case 'hydrate': {
      const items: Record<string, ItemRow> = {};
      for (const item of action.items) items[item.id] = item;
      const order = sortItems(action.items);
      return {
        ...state,
        items,
        order,
        selectedId: state.selectedId && items[state.selectedId] ? state.selectedId : order[0] ?? null,
      };
    }
    case 'apply_change': {
      const next = { ...state.items };
      let { selectedId } = state;
      if (action.change.type === 'item_resolved') {
        delete next[action.change.id];
        if (selectedId === action.change.id) selectedId = null;
      } else {
        next[action.change.item.id] = action.change.item;
      }
      const order = sortItems(Object.values(next));
      if (!selectedId) selectedId = order[0] ?? null;
      return { ...state, items: next, order, selectedId };
    }
    case 'set_status':
      return { ...state, status: action.status, lastError: action.error ?? null };
    case 'select': {
      const visible = state.order
        .map((id) => state.items[id])
        .filter((it): it is ItemRow => !!it && it.state !== 'resolved' && it.state !== 'snoozed');
      if (visible.length === 0) return state;
      const currentIdx = Math.max(0, visible.findIndex((it) => it.id === state.selectedId));
      const nextIdx = Math.min(visible.length - 1, Math.max(0, currentIdx + action.delta));
      return { ...state, selectedId: visible[nextIdx].id };
    }
    case 'select_id':
      return { ...state, selectedId: action.id };
    case 'open_snooze_menu':
      return { ...state, snoozeMenuFor: action.id };
    case 'close_snooze_menu':
      return { ...state, snoozeMenuFor: null };
    case 'open_detail':
      return { ...state, detailFor: action.id };
    case 'close_detail':
      return { ...state, detailFor: null };
  }
}

const initialState: State = {
  items: {},
  order: [],
  selectedId: null,
  status: 'connecting',
  snoozeMenuFor: null,
  detailFor: null,
  lastError: null,
};

const SNOOZE_OPTIONS: Array<{ label: string; key: string; seconds: number }> = [
  { label: '15m', key: '1', seconds: 15 * 60 },
  { label: '1h', key: '2', seconds: 60 * 60 },
  { label: '4h', key: '3', seconds: 4 * 60 * 60 },
  { label: 'tomorrow 9am', key: '4', seconds: -1 },
];

function tomorrow9amUnix(): number {
  const t = new Date();
  t.setDate(t.getDate() + 1);
  t.setHours(9, 0, 0, 0);
  return Math.floor(t.getTime() / 1000);
}

function ageLabel(updatedAtUnix: number): string {
  const ageSec = Math.max(0, Math.floor(Date.now() / 1000) - updatedAtUnix);
  if (ageSec < 60) return `${ageSec}s`;
  if (ageSec < 60 * 60) return `${Math.floor(ageSec / 60)}m`;
  if (ageSec < 60 * 60 * 24) return `${Math.floor(ageSec / 3600)}h`;
  return `${Math.floor(ageSec / 86400)}d`;
}

const kindIcon: Record<ItemRow['kind'], string> = {
  needs_input: '⚠',
  error: '✗',
  stuck: '⏳',
  done: '✓',
  working_milestone: '\u{1F916}',
};

function priorityColor(priority: number): string {
  if (priority >= 80) return '#e06b6b';
  if (priority >= 60) return '#e0c46b';
  if (priority >= 30) return '#6ba0e0';
  return '#6b6b6b';
}

const InboxPane: React.FC<InboxPaneProps> = ({ title, isActive, onAddTab }) => {
  const [state, dispatch] = useReducer(reducer, initialState);
  const clientRef = useRef<ClaudemonItemsClient | null>(null);
  const sessionsRef = useRef<ClaudemonSessionsClient | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const selectedRowRef = useRef<HTMLDivElement>(null);

  if (clientRef.current === null) {
    clientRef.current = new ClaudemonItemsClient();
  }
  if (sessionsRef.current === null) {
    sessionsRef.current = new ClaudemonSessionsClient();
  }
  const client = clientRef.current;
  const sessionsClient = sessionsRef.current;

  // Initial load + SSE subscription
  useEffect(() => {
    let cancelled = false;
    client
      .list()
      .then((items) => {
        if (cancelled) return;
        dispatch({ type: 'hydrate', items });
        dispatch({ type: 'set_status', status: 'connected' });
      })
      .catch((err) => {
        if (cancelled) return;
        dispatch({ type: 'set_status', status: 'error', error: String(err) });
      });
    const cleanup = client.subscribe(
      (change) => dispatch({ type: 'apply_change', change }),
      () => dispatch({ type: 'set_status', status: 'error', error: 'stream interrupted' }),
    );
    return () => {
      cancelled = true;
      cleanup();
    };
  }, [client]);

  // Keep the selected row in view when the user navigates
  useEffect(() => {
    selectedRowRef.current?.scrollIntoView({ block: 'nearest' });
  }, [state.selectedId]);

  const applyAction = useCallback(
    async (id: string, action: ItemAction) => {
      try {
        await client.action(id, action);
      } catch (err) {
        dispatch({ type: 'set_status', status: 'error', error: String(err) });
      }
    },
    [client],
  );

  const visible = useMemo(() => visibleItems(state), [state]);

  // Keyboard handler attached to the pane root; only fires when this pane
  // has focus, so it doesn't fight other panes' keymaps.
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (state.snoozeMenuFor) {
        const choice = SNOOZE_OPTIONS.find((o) => o.key === e.key);
        if (choice) {
          e.preventDefault();
          const until = choice.seconds === -1 ? tomorrow9amUnix() : Math.floor(Date.now() / 1000) + choice.seconds;
          applyAction(state.snoozeMenuFor, { action: 'snooze_until', until });
          dispatch({ type: 'close_snooze_menu' });
          return;
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          dispatch({ type: 'close_snooze_menu' });
          return;
        }
        return;
      }

      // Overlay open: swallow most keys; the overlay has its own handler.
      if (state.detailFor) return;

      const sel = state.selectedId;
      switch (e.key) {
        case 'j':
        case 'ArrowDown':
          e.preventDefault();
          dispatch({ type: 'select', delta: 1 });
          break;
        case 'k':
        case 'ArrowUp':
          e.preventDefault();
          dispatch({ type: 'select', delta: -1 });
          break;
        case 'Enter':
          if (sel) {
            e.preventDefault();
            dispatch({ type: 'open_detail', id: sel });
          }
          break;
        case 'o':
          if (sel && onAddTab) {
            e.preventDefault();
            const item = state.items[sel];
            if (item) {
              onAddTab('claude', undefined, item.session_name, undefined, undefined, undefined, item.session_id);
            }
          }
          break;
        case 'e':
          if (sel) {
            e.preventDefault();
            applyAction(sel, { action: 'archive' });
          }
          break;
        case 's':
          if (sel) {
            e.preventDefault();
            dispatch({ type: 'open_snooze_menu', id: sel });
          }
          break;
        case '!':
          if (sel) {
            e.preventDefault();
            const item = state.items[sel];
            applyAction(sel, { action: item?.flagged ? 'unflag' : 'flag' });
          }
          break;
      }
    },
    [state, applyAction, onAddTab],
  );

  // Auto-focus the pane root so the keymap fires immediately. Also re-focus
  // when this pane becomes active (e.g., user navigated back).
  useEffect(() => {
    if (isActive) containerRef.current?.focus();
  }, [isActive]);

  const counts = useMemo(() => {
    const all = Object.values(state.items);
    return {
      visible: visible.length,
      snoozed: all.filter((i) => i.state === 'snoozed').length,
      flagged: all.filter((i) => i.flagged && i.state !== 'resolved').length,
    };
  }, [state.items, visible.length]);

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      onKeyDown={onKeyDown}
      style={{
        height: '100%',
        outline: 'none',
        display: 'flex',
        flexDirection: 'column',
        backgroundColor: 'var(--wks-bg-base, #14141a)',
        color: 'var(--wks-text-primary, #d8d8e0)',
        fontFamily: 'inherit',
        fontSize: '13px',
      }}
    >
      <div
        style={{
          padding: '8px 12px',
          borderBottom: '1px solid var(--wks-border, #2a2a35)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
        }}
      >
        <div style={{ fontWeight: 600 }}>
          {title} <span style={{ opacity: 0.5, fontWeight: 400, marginLeft: 8 }}>
            {counts.visible} active{counts.snoozed ? ` · ${counts.snoozed} snoozed` : ''}
            {counts.flagged ? ` · ${counts.flagged} flagged` : ''}
          </span>
        </div>
        <div style={{ fontSize: '11px', opacity: 0.6 }}>
          {state.status === 'connecting' && 'connecting…'}
          {state.status === 'connected' && '● live'}
          {state.status === 'error' && (state.lastError || 'error')}
        </div>
      </div>

      <div style={{ flex: 1, overflowY: 'auto' }}>
        {visible.length === 0 && (
          <div
            style={{
              padding: '40px 16px',
              textAlign: 'center',
              color: 'var(--wks-text-muted, #6a6a78)',
            }}
          >
            Inbox zero.
          </div>
        )}
        {visible.map((item) => {
          const selected = item.id === state.selectedId;
          return (
            <div
              key={item.id}
              ref={selected ? selectedRowRef : undefined}
              onClick={() => dispatch({ type: 'select_id', id: item.id })}
              style={{
                display: 'grid',
                gridTemplateColumns: '4px 14px 140px 16px 1fr auto',
                gap: 8,
                alignItems: 'center',
                padding: '6px 8px',
                cursor: 'pointer',
                backgroundColor: selected
                  ? 'var(--wks-bg-hover, #25253a)'
                  : 'transparent',
                borderLeft: selected ? '2px solid var(--wks-accent, #7c7cf0)' : '2px solid transparent',
              }}
            >
              <div
                style={{
                  width: 4,
                  height: 24,
                  backgroundColor: priorityColor(item.priority),
                  borderRadius: 2,
                }}
              />
              <div style={{ fontSize: 14 }}>
                {item.flagged ? '⚑' : item.state === 'unread' ? '●' : '○'}
              </div>
              <div
                style={{
                  fontWeight: 500,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
                title={item.session_name}
              >
                {item.session_name}
              </div>
              <div title={item.kind}>{kindIcon[item.kind]}</div>
              <div
                style={{
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  opacity: 0.85,
                }}
                title={item.summary || ''}
              >
                {item.summary ?? <span style={{ opacity: 0.5 }}>(no summary)</span>}
              </div>
              <div style={{ fontSize: 11, opacity: 0.55, marginLeft: 8 }}>
                {ageLabel(item.updated_at)}
              </div>
            </div>
          );
        })}
      </div>

      <div
        style={{
          padding: '6px 12px',
          borderTop: '1px solid var(--wks-border, #2a2a35)',
          fontSize: 11,
          opacity: 0.6,
          display: 'flex',
          gap: 12,
        }}
      >
        {state.snoozeMenuFor ? (
          <>
            Snooze:&nbsp;
            {SNOOZE_OPTIONS.map((o) => (
              <span key={o.key}>
                [{o.key}] {o.label}
              </span>
            ))}
            <span>[esc] cancel</span>
          </>
        ) : (
          <>
            <span>[j/k] navigate</span>
            <span>[enter] open</span>
            {onAddTab && <span>[o] session</span>}
            <span>[e] archive</span>
            <span>[s] snooze</span>
            <span>[!] flag</span>
          </>
        )}
      </div>
      {state.detailFor && state.items[state.detailFor] && (
        <ItemDetailOverlay
          item={state.items[state.detailFor]}
          itemsClient={client}
          sessionsClient={sessionsClient}
          onClose={() => {
            dispatch({ type: 'close_detail' });
            containerRef.current?.focus();
          }}
          onSnoozeMenu={(id) => dispatch({ type: 'open_snooze_menu', id })}
          onOpenSession={
            onAddTab
              ? (item) =>
                  onAddTab(
                    'claude',
                    undefined,
                    item.session_name,
                    undefined,
                    undefined,
                    undefined,
                    item.session_id,
                  )
              : undefined
          }
        />
      )}
    </div>
  );
};

export default InboxPane;
