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

type SectionId = 'needs_attention' | 'working' | 'snoozed';

interface State {
  items: Record<string, ItemRow>;
  order: string[];
  selectedId: string | null;
  /** Multi-select set for bulk actions (Space adds/removes, `a` archives all). */
  multiSelect: Set<string>;
  status: Status;
  snoozeMenuFor: string | null;
  detailFor: string | null;
  collapsed: Record<SectionId, boolean>;
  searchOpen: boolean;
  query: string;
  replyFor: string | null;
  replyText: string;
  replyError: string | null;
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
  | { type: 'close_detail' }
  | { type: 'toggle_section'; section: SectionId }
  | { type: 'open_search' }
  | { type: 'close_search' }
  | { type: 'set_query'; query: string }
  | { type: 'toggle_multi'; id: string }
  | { type: 'clear_multi' }
  | { type: 'open_reply'; id: string }
  | { type: 'close_reply' }
  | { type: 'set_reply_text'; text: string }
  | { type: 'set_reply_error'; error: string | null };

function sortItems(items: ItemRow[]): string[] {
  return [...items]
    .sort((a, b) => {
      if (a.flagged !== b.flagged) return a.flagged ? -1 : 1;
      if (a.priority !== b.priority) return b.priority - a.priority;
      return b.updated_at - a.updated_at;
    })
    .map((i) => i.id);
}

function bucketOf(item: ItemRow): SectionId | null {
  if (item.state === 'resolved') return null;
  if (item.state === 'snoozed') return 'snoozed';
  if (item.priority >= 70) return 'needs_attention';
  return 'working';
}

function bucketedItems(state: State): Record<SectionId, ItemRow[]> {
  const out: Record<SectionId, ItemRow[]> = {
    needs_attention: [],
    working: [],
    snoozed: [],
  };
  const q = state.query.trim().toLowerCase();
  for (const id of state.order) {
    const item = state.items[id];
    if (!item) continue;
    if (q && !matchesQuery(item, q)) continue;
    const bucket = bucketOf(item);
    if (bucket) out[bucket].push(item);
  }
  return out;
}

function matchesQuery(item: ItemRow, q: string): boolean {
  return (
    item.session_name.toLowerCase().includes(q) ||
    item.session_project.toLowerCase().includes(q) ||
    (item.summary?.toLowerCase().includes(q) ?? false) ||
    item.kind.includes(q)
  );
}

/**
 * Items the user can actually navigate to with j/k right now — items in
 * non-collapsed sections, in their on-screen order.
 */
function navigableItems(state: State): ItemRow[] {
  const buckets = bucketedItems(state);
  const order: SectionId[] = ['needs_attention', 'working', 'snoozed'];
  return order.flatMap((sec) => (state.collapsed[sec] ? [] : buckets[sec]));
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
      const visible = navigableItems(state);
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
    case 'toggle_section':
      return {
        ...state,
        collapsed: { ...state.collapsed, [action.section]: !state.collapsed[action.section] },
      };
    case 'open_search':
      return { ...state, searchOpen: true };
    case 'close_search':
      return { ...state, searchOpen: false, query: '' };
    case 'set_query':
      return { ...state, query: action.query };
    case 'toggle_multi': {
      const next = new Set(state.multiSelect);
      if (next.has(action.id)) {
        next.delete(action.id);
      } else {
        next.add(action.id);
      }
      return { ...state, multiSelect: next };
    }
    case 'clear_multi':
      return { ...state, multiSelect: new Set() };
    case 'open_reply':
      return { ...state, replyFor: action.id, replyText: '', replyError: null };
    case 'close_reply':
      return { ...state, replyFor: null, replyText: '', replyError: null };
    case 'set_reply_text':
      return { ...state, replyText: action.text };
    case 'set_reply_error':
      return { ...state, replyError: action.error };
  }
}

const initialState: State = {
  items: {},
  order: [],
  selectedId: null,
  multiSelect: new Set<string>(),
  status: 'connecting',
  snoozeMenuFor: null,
  detailFor: null,
  collapsed: {
    needs_attention: false,
    working: false,
    snoozed: true,
  },
  searchOpen: false,
  query: '',
  replyFor: null,
  replyText: '',
  replyError: null,
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
      .list({ include_snoozed: true })
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

  const submitReply = useCallback(
    async (id: string, text: string) => {
      const trimmed = text.trim();
      if (!trimmed) return;
      const item = state.items[id];
      if (!item) return;
      try {
        await sessionsClient.sendMessage(item.session_id, trimmed);
        // Auto-snooze on next_event so the item leaves the inbox until the
        // session reacts; spec §7.
        await client.action(id, { action: 'snooze_on_event', on: 'next_event' });
        dispatch({ type: 'close_reply' });
      } catch (err) {
        dispatch({ type: 'set_reply_error', error: String(err) });
      }
    },
    [client, sessionsClient, state.items],
  );

  const buckets = useMemo(() => bucketedItems(state), [state]);
  const visibleCount = navigableItems(state).length;

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

      // Search input has focus: Esc closes; everything else passes through
      // to the input element's own handlers.
      if (state.searchOpen) {
        if (e.key === 'Escape') {
          e.preventDefault();
          dispatch({ type: 'close_search' });
        }
        return;
      }

      // Reply input has focus: input handles its own keys, root catches Esc.
      if (state.replyFor) {
        if (e.key === 'Escape') {
          e.preventDefault();
          dispatch({ type: 'close_reply' });
        }
        return;
      }

      // Open search on /
      if (e.key === '/') {
        e.preventDefault();
        dispatch({ type: 'open_search' });
        return;
      }

      const sel = state.selectedId;
      const multi = state.multiSelect;
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
        case 'Escape':
          if (multi.size > 0) {
            e.preventDefault();
            dispatch({ type: 'clear_multi' });
          }
          break;
        case ' ': // Space: toggle multi-select for the cursor item
          if (sel) {
            e.preventDefault();
            dispatch({ type: 'toggle_multi', id: sel });
          }
          break;
        case 'a':
          // Bulk archive all multi-selected items. Runs sequentially so
          // we don't overload the daemon with a parallel POST burst.
          if (multi.size > 0) {
            e.preventDefault();
            (async () => {
              for (const id of multi) {
                await applyAction(id, { action: 'archive' });
              }
              dispatch({ type: 'clear_multi' });
            })();
          }
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
        case 'r':
          if (sel) {
            e.preventDefault();
            dispatch({ type: 'open_reply', id: sel });
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
      visible: visibleCount,
      snoozed: all.filter((i) => i.state === 'snoozed').length,
      flagged: all.filter((i) => i.flagged && i.state !== 'resolved').length,
    };
  }, [state.items, visibleCount]);

  const renderRow = (item: ItemRow) => {
    const selected = item.id === state.selectedId;
    const isMulti = state.multiSelect.has(item.id);
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
          backgroundColor: isMulti
            ? 'var(--wks-bg-active, #2a2a45)'
            : selected
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
          {isMulti ? '☑' : item.flagged ? '⚑' : item.state === 'unread' ? '●' : '○'}
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
  };

  const renderSection = (id: SectionId, label: string, items: ItemRow[]) => {
    if (items.length === 0) return null;
    const isCollapsed = state.collapsed[id];
    return (
      <div key={id}>
        <div
          onClick={() => dispatch({ type: 'toggle_section', section: id })}
          style={{
            padding: '6px 12px',
            fontSize: 11,
            textTransform: 'uppercase',
            letterSpacing: '0.05em',
            color: 'var(--wks-text-muted, #888892)',
            cursor: 'pointer',
            userSelect: 'none',
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            borderTop: '1px solid var(--wks-border, #2a2a35)',
          }}
          title={`Toggle ${label}`}
        >
          <span style={{ display: 'inline-block', width: 10 }}>{isCollapsed ? '▸' : '▾'}</span>
          <span>{label}</span>
          <span style={{ opacity: 0.6 }}>· {items.length}</span>
        </div>
        {!isCollapsed && items.map(renderRow)}
      </div>
    );
  };

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

      {state.searchOpen && (
        <div
          style={{
            padding: '6px 12px',
            borderBottom: '1px solid var(--wks-border, #2a2a35)',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
          }}
        >
          <span style={{ opacity: 0.6, fontSize: 12 }}>/</span>
          <input
            autoFocus
            value={state.query}
            placeholder="search session, summary, kind…"
            onChange={(e) => dispatch({ type: 'set_query', query: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault();
                dispatch({ type: 'close_search' });
                containerRef.current?.focus();
              }
            }}
            style={{
              flex: 1,
              background: 'transparent',
              color: 'var(--wks-text-primary, #d8d8e0)',
              border: 'none',
              outline: 'none',
              fontFamily: 'inherit',
              fontSize: 13,
            }}
            aria-label="Inbox search"
          />
          <button
            onClick={() => {
              dispatch({ type: 'close_search' });
              containerRef.current?.focus();
            }}
            style={{
              background: 'transparent',
              border: 'none',
              color: 'var(--wks-text-muted, #888892)',
              cursor: 'pointer',
              fontSize: 11,
            }}
          >
            ✕
          </button>
        </div>
      )}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {Object.values(state.items).filter((i) => i.state !== 'resolved').length === 0 && (
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
        {state.query.trim() && visibleCount === 0 && Object.values(state.items).filter((i) => i.state !== 'resolved').length > 0 && (
          <div
            style={{
              padding: '24px 16px',
              textAlign: 'center',
              color: 'var(--wks-text-muted, #6a6a78)',
              fontSize: 12,
            }}
          >
            No matches.
          </div>
        )}
        {renderSection('needs_attention', 'Needs attention', buckets.needs_attention)}
        {renderSection('working', 'Working', buckets.working)}
        {renderSection('snoozed', 'Snoozed', buckets.snoozed)}
      </div>

      {state.replyFor && state.items[state.replyFor] && (
        <div
          style={{
            padding: '8px 12px',
            borderTop: '1px solid var(--wks-border, #2a2a35)',
            backgroundColor: 'var(--wks-bg-elevated, #1c1c26)',
          }}
        >
          <div style={{ fontSize: 11, opacity: 0.6, marginBottom: 4 }}>
            Reply to {state.items[state.replyFor].session_name}
          </div>
          <input
            autoFocus
            value={state.replyText}
            placeholder="type a message and press enter…"
            onChange={(e) => dispatch({ type: 'set_reply_text', text: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                submitReply(state.replyFor!, state.replyText);
              } else if (e.key === 'Escape') {
                e.preventDefault();
                dispatch({ type: 'close_reply' });
                containerRef.current?.focus();
              }
            }}
            style={{
              width: '100%',
              background: 'var(--wks-bg-input, #14141a)',
              color: 'var(--wks-text-primary, #d8d8e0)',
              border: '1px solid var(--wks-border, #2a2a35)',
              borderRadius: 4,
              padding: '6px 8px',
              fontFamily: 'inherit',
              fontSize: 13,
              outline: 'none',
            }}
            aria-label="Reply input"
          />
          {state.replyError && (
            <div style={{ marginTop: 4, fontSize: 11, color: '#e06b6b' }}>
              {state.replyError}
            </div>
          )}
        </div>
      )}
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
            {state.multiSelect.size > 0 ? (
              <>
                <span>{state.multiSelect.size} selected</span>
                <span>[a] archive all</span>
                <span>[space] toggle</span>
                <span>[esc] clear</span>
              </>
            ) : state.replyFor ? (
              <>
                <span>[enter] send</span>
                <span>[esc] cancel</span>
              </>
            ) : (
              <>
                <span>[j/k] navigate</span>
                <span>[enter] open</span>
                {onAddTab && <span>[o] session</span>}
                <span>[e] archive</span>
                <span>[s] snooze</span>
                <span>[r] reply</span>
                <span>[!] flag</span>
                <span>[space] select</span>
                <span>[/] search</span>
              </>
            )}
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
