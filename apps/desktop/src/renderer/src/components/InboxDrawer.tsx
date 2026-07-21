import React, { useCallback, useEffect, useMemo, useRef } from 'react';
import { CheckCircle2, X } from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useAttention } from '../contexts/AttentionContext';
import { AttentionCard } from './attention/AttentionCard';
import { SNOOZE_MINUTES } from '../contexts/AttentionContext';
import { captionInsetTop } from '../lib/layoutUtils';
import { useConfig } from '../hooks/useConfig';
import { DEFAULT_SHORTCUTS } from '../hooks/configDefaults';
import { eventMatchesCombo, digitFromRangeEvent, formatBinding } from '../lib/shortcuts';

const DRAWER_WIDTH = 440;

/**
 * The Triage Inbox — a top-level right-side drawer reachable from ANY agent
 * (not buried in a workspace). It is a pure projection of the attention feed:
 * zero live panes, so it can never remount a terminal/webview/Claude viewer.
 * You clear it top-down like email; resolution happens in place via the
 * by-sessionId actions on AttentionContext.
 */
const InboxDrawer: React.FC = () => {
  const {
    inboxOpen,
    closeInbox,
    feed,
    counts,
    selectedItem,
    moveSelection,
    setSelectedSig,
    approve,
    answer,
    dismiss,
    snooze,
    openAgent,
    inboxFilter,
    setInboxFilter,
  } = useAttention();
  const reviewCount = Math.max(0, counts.total - counts.needsYou);

  // Drawer-scoped keybindings (inbox-*), remappable in Settings → Keybindings.
  // Defaults merged under user overrides so a partial saved map still binds.
  const { config } = useConfig();
  const sc = useMemo(
    () => ({ ...DEFAULT_SHORTCUTS, ...(config.keybindings?.shortcuts ?? {}) }),
    [config.keybindings?.shortcuts],
  );

  // Bulk triage: dismiss every non-blocking item in the CURRENT feed in one go.
  // Approvals/questions are deliberately excluded — bulk-hiding a card whose
  // agent is still blocked would just bury the block.
  const reviewedInFeed = feed.filter((it) => it.kind !== 'approval' && it.kind !== 'question');
  const clearReviewed = useCallback(() => {
    for (const it of feed) {
      if (it.kind !== 'approval' && it.kind !== 'question') dismiss(it.signature);
    }
  }, [feed, dismiss]);
  const TABS: { key: typeof inboxFilter; label: string; count: number }[] = [
    { key: 'all', label: 'All', count: counts.total },
    { key: 'needs', label: 'Needs you', count: counts.needsYou },
    { key: 'review', label: 'Review', count: reviewCount },
  ];
  const listRef = useRef<HTMLDivElement>(null);

  // Windowed feed: only the visible cards (plus a little overscan) are in the
  // DOM, so a 100+-item inbox stays smooth. Heights vary by item type
  // (approval / question / summary), so we measure each card dynamically.
  const virtualizer = useVirtualizer({
    count: feed.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => 112,
    overscan: 6,
  });

  // Keep the selected card scrolled into view as you j/k through the feed.
  useEffect(() => {
    if (!inboxOpen || !selectedItem) return;
    const idx = feed.findIndex((it) => it.signature === selectedItem.signature);
    if (idx >= 0) virtualizer.scrollToIndex(idx, { align: 'auto' });
  }, [inboxOpen, selectedItem, feed, virtualizer]);

  // Keyboard triage — only while the drawer is open and you're not typing into
  // an input (e.g. the question picker's custom-answer field).
  useEffect(() => {
    if (!inboxOpen) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) {
        if (e.key === 'Escape') (t as HTMLInputElement).blur();
        return;
      }
      const it = selectedItem;
      const stop = () => {
        e.preventDefault();
        e.stopPropagation();
      };

      // Escape (close), arrows (move), and Enter (act on the selected card) are
      // fixed; the letter keys are remappable inbox-* bindings.
      if (e.key === 'Escape') {
        stop();
        closeInbox();
        return;
      }
      if (eventMatchesCombo(e, sc['inbox-move-down']) || e.key === 'ArrowDown') {
        stop();
        moveSelection(1);
        return;
      }
      if (eventMatchesCombo(e, sc['inbox-move-up']) || e.key === 'ArrowUp') {
        stop();
        moveSelection(-1);
        return;
      }
      if (eventMatchesCombo(e, sc['inbox-clear-reviewed'])) {
        stop();
        clearReviewed();
        return;
      }
      if (!it) return;

      if (eventMatchesCombo(e, sc['inbox-open'])) {
        stop();
        openAgent(it.agentId);
        return;
      }
      if (eventMatchesCombo(e, sc['inbox-dismiss'])) {
        stop();
        dismiss(it.signature);
        return;
      }
      if (eventMatchesCombo(e, sc['inbox-snooze'])) {
        stop();
        snooze(it.signature, SNOOZE_MINUTES);
        return;
      }

      if (it.payload.type === 'approval') {
        if (eventMatchesCombo(e, sc['inbox-approve-yes']) || e.key === 'Enter') {
          stop();
          approve(it, 'yes');
          return;
        }
        if (eventMatchesCombo(e, sc['inbox-approve-no'])) {
          stop();
          approve(it, 'no');
          return;
        }
      }
      if (it.payload.type === 'question') {
        const n = digitFromRangeEvent(e, sc['inbox-answer']);
        if (n !== null && n <= (it.payload.questions[0]?.options.length ?? 0)) {
          stop();
          answer(it, { option: n });
          return;
        }
        if (e.key === 'Enter') {
          stop();
          openAgent(it.agentId);
          return;
        }
      }
      if (it.payload.type === 'summary' && e.key === 'Enter') {
        stop();
        openAgent(it.agentId);
        return;
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [
    inboxOpen,
    selectedItem,
    moveSelection,
    closeInbox,
    openAgent,
    dismiss,
    snooze,
    approve,
    answer,
    clearReviewed,
    sc,
  ]);

  return (
    <>
      {/* Backdrop */}
      <div
        onClick={closeInbox}
        style={{
          position: 'fixed',
          inset: 0,
          zIndex: 300,
          background: 'rgba(0,0,0,0.28)',
          opacity: inboxOpen ? 1 : 0,
          pointerEvents: inboxOpen ? 'auto' : 'none',
          transition: 'opacity 0.16s ease',
        }}
      />
      <div
        style={{
          position: 'fixed',
          top: 0,
          right: 0,
          bottom: 0,
          width: DRAWER_WIDTH,
          maxWidth: '92vw',
          zIndex: 301,
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
          borderLeft: '1px solid var(--wks-glass-border)',
          boxShadow: '-12px 0 40px var(--wks-shadow)',
          transform: inboxOpen ? 'translateX(0)' : 'translateX(100%)',
          transition: 'transform 0.18s cubic-bezier(0.32, 0.72, 0, 1)',
        }}
      >
        {/* Header — pad past the Windows caption buttons so the close ✕ clears them. */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: `${14 + captionInsetTop()}px 16px 10px`,
          }}
        >
          <div
            style={{
              fontSize: '0.95rem',
              fontWeight: 700,
              color: 'var(--wks-text-primary)',
              letterSpacing: '-0.01em',
            }}
          >
            Inbox
          </div>
          {counts.needsYou > 0 ? (
            <span style={{ fontSize: '0.72rem', fontWeight: 700, color: 'var(--wks-warning)' }}>
              {counts.needsYou} need you
            </span>
          ) : counts.total > 0 ? (
            <span style={{ fontSize: '0.72rem', fontWeight: 600, color: 'var(--wks-success)' }}>
              {counts.total} to review
            </span>
          ) : (
            <span style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>all clear</span>
          )}
          <div style={{ flex: 1 }} />
          <button onClick={closeInbox} title="Close (Esc)" style={closeBtn}>
            <X size={12} strokeWidth={2} />
          </button>
        </div>

        {/* Filter tabs — All / Needs you / Review. `feed` is filtered in context,
            so keyboard triage operates on exactly what's shown. */}
        <div style={{ display: 'flex', gap: 4, padding: '0 16px 8px' }}>
          {TABS.map((t) => {
            const active = inboxFilter === t.key;
            return (
              <button
                key={t.key}
                onClick={() => setInboxFilter(t.key)}
                style={{
                  flex: 1,
                  padding: '4px 6px',
                  borderRadius: 6,
                  cursor: 'pointer',
                  fontSize: '0.7rem',
                  fontWeight: 600,
                  fontFamily: 'inherit',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 5,
                  border: active
                    ? '1px solid var(--wks-accent)'
                    : '1px solid var(--wks-border-input)',
                  background: active ? 'var(--wks-accent-bg)' : 'transparent',
                  color: active ? 'var(--wks-accent-text)' : 'var(--wks-text-tertiary)',
                }}
              >
                {t.label}
                {t.count > 0 && (
                  <span style={{ fontSize: '0.64rem', fontWeight: 700, opacity: 0.8 }}>
                    {t.count}
                  </span>
                )}
              </button>
            );
          })}
        </div>

        {/* Hint strip + bulk actions */}
        <div
          style={{
            padding: '0 16px 10px',
            fontSize: '0.66rem',
            color: 'var(--wks-text-faint)',
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            flexWrap: 'wrap',
          }}
        >
          <Hint
            k={`${formatBinding(sc['inbox-move-down'] ?? '')}/${formatBinding(sc['inbox-move-up'] ?? '')}`}
            t="move"
          />
          <Hint
            k={`${formatBinding(sc['inbox-approve-yes'] ?? '')}/${formatBinding(sc['inbox-approve-no'] ?? '')}`}
            t="approve"
          />
          <Hint k={formatBinding(sc['inbox-answer'] ?? '')} t="answer" />
          <Hint k={formatBinding(sc['inbox-open'] ?? '')} t="open" />
          <Hint k={formatBinding(sc['inbox-dismiss'] ?? '')} t="dismiss" />
          <Hint k={formatBinding(sc['inbox-snooze'] ?? '')} t="snooze" />
          {reviewedInFeed.length > 1 && (
            <button
              onClick={clearReviewed}
              title={`Dismiss every reviewed (non-blocking) item shown (${formatBinding(sc['inbox-clear-reviewed'] ?? '')})`}
              style={{
                marginLeft: 'auto',
                fontSize: '0.66rem',
                fontWeight: 600,
                fontFamily: 'inherit',
                cursor: 'pointer',
                border: '1px solid var(--wks-border-input)',
                borderRadius: 6,
                padding: '2px 9px',
                background: 'transparent',
                color: 'var(--wks-text-secondary)',
                whiteSpace: 'nowrap',
              }}
            >
              Clear {reviewedInFeed.length} reviewed
            </button>
          )}
        </div>

        {/* Feed (windowed) */}
        <div ref={listRef} style={{ flex: 1, overflowY: 'auto', padding: '4px 14px 18px' }}>
          {feed.length === 0 ? (
            <div style={{ marginTop: 64, textAlign: 'center', color: 'var(--wks-text-faint)' }}>
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'center',
                  marginBottom: 8,
                  color: 'var(--wks-success)',
                }}
              >
                <CheckCircle2 size={30} strokeWidth={1.75} />
              </div>
              <div
                style={{ fontSize: '0.82rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}
              >
                Inbox zero
              </div>
              <div
                style={{
                  fontSize: '0.72rem',
                  marginTop: 4,
                  lineHeight: 1.5,
                  maxWidth: 240,
                  marginInline: 'auto',
                }}
              >
                No agent needs you right now. Approvals, questions, and finished runs will surface
                here.
              </div>
            </div>
          ) : (
            <div
              style={{ position: 'relative', width: '100%', height: virtualizer.getTotalSize() }}
            >
              {virtualizer.getVirtualItems().map((v) => {
                const it = feed[v.index];
                return (
                  <div
                    key={it.signature}
                    data-index={v.index}
                    ref={virtualizer.measureElement}
                    style={{
                      position: 'absolute',
                      top: 0,
                      left: 0,
                      width: '100%',
                      transform: `translateY(${v.start}px)`,
                      paddingBottom: 12,
                    }}
                  >
                    <AttentionCard item={it} selected={selectedItem?.signature === it.signature} />
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </>
  );
};

const closeBtn: React.CSSProperties = {
  width: 26,
  height: 26,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  border: '1px solid var(--wks-glass-border)',
  borderRadius: 6,
  cursor: 'pointer',
  background: 'var(--wks-bg-surface)',
  color: 'var(--wks-text-secondary)',
};

const Hint: React.FC<{ k: string; t: string }> = ({ k, t }) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
    <kbd
      style={{
        fontSize: '0.62rem',
        color: 'var(--wks-text-secondary)',
        border: '1px solid var(--wks-glass-border)',
        borderRadius: 3,
        padding: '0 4px',
        fontFamily: 'var(--wks-font-mono)',
      }}
    >
      {k}
    </kbd>
    <span>{t}</span>
  </span>
);

export default InboxDrawer;
