/**
 * L2 item detail overlay (spec §5 L2). Opens over the inbox when the user
 * presses Enter on an item. Three tabs (Decision / Diff / Transcript) plus
 * a single-key actions footer. Closes on Esc.
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ClaudemonItemsClient, ItemAction, ItemRow } from '../lib/claudemonItems';
import {
  ClaudemonSessionsClient,
  Transcript,
  TranscriptMessage,
} from '../lib/claudemonSessions';

interface ItemDetailOverlayProps {
  item: ItemRow;
  itemsClient: ClaudemonItemsClient;
  sessionsClient: ClaudemonSessionsClient;
  onClose: () => void;
  onSnoozeMenu: (id: string) => void;
  /** Optional: spawn an L3 Claude pane attached to the item's session. */
  onOpenSession?: (item: ItemRow) => void;
}

type Tab = 'decision' | 'diff' | 'transcript';

function ageLabel(updatedAtUnix: number): string {
  const ageSec = Math.max(0, Math.floor(Date.now() / 1000) - updatedAtUnix);
  if (ageSec < 60) return `${ageSec}s`;
  if (ageSec < 60 * 60) return `${Math.floor(ageSec / 60)}m`;
  if (ageSec < 60 * 60 * 24) return `${Math.floor(ageSec / 3600)}h`;
  return `${Math.floor(ageSec / 86400)}d`;
}

function priorityColor(priority: number): string {
  if (priority >= 80) return '#e06b6b';
  if (priority >= 60) return '#e0c46b';
  if (priority >= 30) return '#6ba0e0';
  return '#6b6b6b';
}

function messagePreview(msg: TranscriptMessage): string {
  if (typeof msg.content === 'string') return msg.content;
  if (Array.isArray(msg.content)) {
    return msg.content
      .map((block: any) => {
        if (typeof block === 'string') return block;
        if (block?.type === 'text' && typeof block.text === 'string') return block.text;
        if (block?.type === 'tool_use') return `[tool: ${block.name ?? '?'}]`;
        if (block?.type === 'tool_result') return `[result]`;
        return '';
      })
      .filter((s: string) => s)
      .join(' ');
  }
  return '';
}

const ItemDetailOverlay: React.FC<ItemDetailOverlayProps> = ({
  item,
  itemsClient,
  sessionsClient,
  onClose,
  onSnoozeMenu,
  onOpenSession,
}) => {
  const [tab, setTab] = useState<Tab>('decision');
  const [transcript, setTranscript] = useState<Transcript | null>(null);
  const [transcriptError, setTranscriptError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    rootRef.current?.focus();
  }, []);

  // Lazy-load transcript only when the user switches to that tab.
  useEffect(() => {
    if (tab !== 'transcript' || transcript || transcriptError) return;
    let cancelled = false;
    sessionsClient
      .getTranscript(item.session_id)
      .then((t) => {
        if (!cancelled) setTranscript(t);
      })
      .catch((err) => {
        if (!cancelled) setTranscriptError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [tab, item.session_id, sessionsClient, transcript, transcriptError]);

  const isNeedsInput = item.kind === 'needs_input';

  const decide = useCallback(
    async (decision: 'yes' | 'no') => {
      if (busy) return;
      setBusy(true);
      setActionError(null);
      try {
        // Best-effort: forward the decision to the session's parked picker.
        // Some PermissionRequests fire without the gate being engaged; in
        // that case the daemon returns 409 and we still archive the item so
        // it leaves the inbox. The user can re-approve in the TUI.
        try {
          await sessionsClient.approve(item.session_id, decision);
        } catch (err) {
          // Surface but don't block archive.
          setActionError(`approve: ${err}`);
        }
        await itemsClient.action(item.id, { action: 'archive' });
        onClose();
      } catch (err) {
        setActionError(String(err));
      } finally {
        setBusy(false);
      }
    },
    [busy, sessionsClient, itemsClient, item.session_id, item.id, onClose],
  );

  const applyAction = useCallback(
    async (action: ItemAction) => {
      if (busy) return;
      setBusy(true);
      setActionError(null);
      try {
        await itemsClient.action(item.id, action);
        onClose();
      } catch (err) {
        setActionError(String(err));
      } finally {
        setBusy(false);
      }
    },
    [busy, itemsClient, item.id, onClose],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === 'Tab') {
        e.preventDefault();
        const order: Tab[] = ['decision', 'diff', 'transcript'];
        const idx = order.indexOf(tab);
        setTab(order[(idx + 1) % order.length]);
        return;
      }
      if (e.key === 'd') {
        e.preventDefault();
        setTab('diff');
        return;
      }
      if (isNeedsInput && e.key === 'y') {
        e.preventDefault();
        decide('yes');
        return;
      }
      if (isNeedsInput && e.key === 'n') {
        e.preventDefault();
        decide('no');
        return;
      }
      if (e.key === 'e') {
        e.preventDefault();
        applyAction({ action: 'archive' });
        return;
      }
      if (e.key === 's') {
        e.preventDefault();
        onSnoozeMenu(item.id);
        onClose();
        return;
      }
      if (e.key === '!') {
        e.preventDefault();
        applyAction({ action: item.flagged ? 'unflag' : 'flag' });
        return;
      }
      if (e.key === 'o' && onOpenSession) {
        e.preventDefault();
        onOpenSession(item);
        onClose();
        return;
      }
    },
    [tab, isNeedsInput, decide, applyAction, onClose, onSnoozeMenu, item, onOpenSession],
  );

  const triggerPayload = useMemo(() => {
    // Decision tab content for needs_input items: pretty-print the tool
    // name + tool_input from the originating event payload if we have it.
    // For Phase 3 we keep this simple — the item only carries
    // triggering_event_id so we surface what's in the summary plus the
    // next_action. A future revision could fetch the event row.
    return null;
  }, []);

  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        backgroundColor: 'rgba(0,0,0,0.55)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 50,
      }}
      onClick={onClose}
    >
      <div
        ref={rootRef}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onClick={(e) => e.stopPropagation()}
        style={{
          width: '92%',
          height: '70%',
          backgroundColor: 'var(--wks-bg-surface, #1a1a22)',
          border: '1px solid var(--wks-border, #2a2a35)',
          borderRadius: 8,
          display: 'flex',
          flexDirection: 'column',
          outline: 'none',
          overflow: 'hidden',
          color: 'var(--wks-text-primary, #d8d8e0)',
          fontSize: 13,
        }}
        aria-label="Item detail"
      >
        {/* Header */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            padding: '10px 16px',
            borderBottom: '1px solid var(--wks-border, #2a2a35)',
          }}
        >
          <div style={{ width: 4, height: 28, backgroundColor: priorityColor(item.priority), borderRadius: 2 }} />
          <div style={{ fontWeight: 600 }}>{item.session_name}</div>
          <div style={{ fontSize: 11, opacity: 0.6 }}>
            {item.session_state} · priority {item.priority} · {ageLabel(item.updated_at)}
          </div>
        </div>

        {/* Headline + context */}
        <div style={{ padding: '14px 16px 6px 16px' }}>
          <div style={{ fontSize: 17, fontWeight: 500 }}>
            {item.summary || <span style={{ opacity: 0.5 }}>(no summary)</span>}
          </div>
          {item.context_paragraph && (
            <div style={{ marginTop: 8, opacity: 0.8 }}>{item.context_paragraph}</div>
          )}
        </div>

        {/* Tabs */}
        <div
          style={{
            display: 'flex',
            gap: 6,
            padding: '8px 16px 0 16px',
            borderBottom: '1px solid var(--wks-border, #2a2a35)',
          }}
        >
          {(['decision', 'diff', 'transcript'] as Tab[]).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              style={{
                background: 'none',
                border: 'none',
                padding: '6px 10px',
                color: tab === t ? 'var(--wks-text-primary, #d8d8e0)' : 'var(--wks-text-muted, #888892)',
                borderBottom: tab === t ? '2px solid var(--wks-accent, #7c7cf0)' : '2px solid transparent',
                cursor: 'pointer',
                fontSize: 12,
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
              }}
            >
              {t}
            </button>
          ))}
        </div>

        {/* Tab body */}
        <div style={{ flex: 1, overflowY: 'auto', padding: '14px 16px' }}>
          {tab === 'decision' && (
            <div>
              <div style={{ marginBottom: 8 }}>
                <span style={{ opacity: 0.6 }}>Suggested action:</span>{' '}
                <span>{item.next_action ?? '—'}</span>
              </div>
              {isNeedsInput ? (
                <div style={{ marginTop: 12, opacity: 0.85 }}>
                  Approve / deny will resolve the session's pending permission
                  request (or fall back to archiving if the picker has already
                  closed in Claude's TUI).
                </div>
              ) : (
                <div style={{ opacity: 0.7 }}>
                  This item is informational — no approve/deny choice required.
                  Use <code>e</code> to archive, <code>s</code> to snooze.
                </div>
              )}
              {triggerPayload}
            </div>
          )}
          {tab === 'diff' && (
            <div style={{ opacity: 0.6 }}>
              Diff view not implemented yet. (Phase 3 placeholder — needs git
              status from the session's worktree.)
            </div>
          )}
          {tab === 'transcript' && (
            <div>
              {transcriptError && (
                <div style={{ color: '#e06b6b' }}>Failed: {transcriptError}</div>
              )}
              {!transcript && !transcriptError && (
                <div style={{ opacity: 0.6 }}>Loading transcript…</div>
              )}
              {transcript && transcript.messages.length === 0 && (
                <div style={{ opacity: 0.6 }}>No messages.</div>
              )}
              {transcript &&
                transcript.messages.slice(-30).map((msg, i) => {
                  const text = messagePreview(msg);
                  if (!text) return null;
                  return (
                    <div key={i} style={{ marginBottom: 10 }}>
                      <div
                        style={{
                          fontSize: 11,
                          textTransform: 'uppercase',
                          letterSpacing: '0.05em',
                          opacity: 0.55,
                          marginBottom: 2,
                        }}
                      >
                        {msg.role}
                      </div>
                      <div style={{ whiteSpace: 'pre-wrap' }}>{text}</div>
                    </div>
                  );
                })}
            </div>
          )}
        </div>

        {/* Action errors */}
        {actionError && (
          <div
            style={{
              padding: '6px 16px',
              borderTop: '1px solid #5a2222',
              backgroundColor: '#3a1818',
              color: '#e0a8a8',
              fontSize: 12,
            }}
          >
            {actionError}
          </div>
        )}

        {/* Footer */}
        <div
          style={{
            padding: '8px 16px',
            borderTop: '1px solid var(--wks-border, #2a2a35)',
            fontSize: 11,
            opacity: 0.7,
            display: 'flex',
            gap: 14,
          }}
        >
          {isNeedsInput && (
            <>
              <span>[y] approve</span>
              <span>[n] deny</span>
            </>
          )}
          <span>[s] snooze</span>
          <span>[!] {item.flagged ? 'unflag' : 'flag'}</span>
          <span>[e] archive</span>
          {onOpenSession && <span>[o] open session</span>}
          <span>[tab] cycle tabs</span>
          <span>[d] diff</span>
          <span>[esc] back</span>
        </div>
      </div>
    </div>
  );
};

export default ItemDetailOverlay;
