import React, { useCallback, useEffect, useRef } from 'react';
import { CheckCircle2 } from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useAttention } from '../contexts/AttentionContext';
import { AttentionCard } from './attention/AttentionCard';
import { SNOOZE_MINUTES } from '../contexts/AttentionContext';
import { captionInsetTop } from '../lib/layoutUtils';

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
    inboxOpen, closeInbox, feed, counts,
    selectedItem, moveSelection, setSelectedSig,
    approve, answer, dismiss, snooze, openAgent,
    inboxFilter, setInboxFilter,
  } = useAttention();
  const reviewCount = Math.max(0, counts.total - counts.needsYou);

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
      const stop = () => { e.preventDefault(); e.stopPropagation(); };

      if (e.key === 'Escape') { stop(); closeInbox(); return; }
      if (e.key === 'j' || e.key === 'ArrowDown') { stop(); moveSelection(1); return; }
      if (e.key === 'k' || e.key === 'ArrowUp') { stop(); moveSelection(-1); return; }
      if (e.key === 'E') { stop(); clearReviewed(); return; }
      if (!it) return;

      if (e.key === 'o') { stop(); openAgent(it.agentId); return; }
      if (e.key === 'e') { stop(); dismiss(it.signature); return; }
      if (e.key === 's') { stop(); snooze(it.signature, SNOOZE_MINUTES); return; }

      if (it.payload.type === 'approval') {
        if (e.key === 'y' || e.key === 'Enter') { stop(); approve(it, 'yes'); return; }
        if (e.key === 'n') { stop(); approve(it, 'no'); return; }
      }
      if (it.payload.type === 'question') {
        const n = parseInt(e.key, 10);
        if (n >= 1 && n <= 9 && n <= (it.payload.questions[0]?.options.length ?? 0)) {
          stop(); answer(it, { option: n }); return;
        }
        if (e.key === 'Enter') { stop(); openAgent(it.agentId); return; }
      }
      if (it.payload.type === 'summary' && e.key === 'Enter') { stop(); openAgent(it.agentId); return; }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [inboxOpen, selectedItem, moveSelection, closeInbox, openAgent, dismiss, snooze, approve, answer, clearReviewed]);

  return (
    <>
      {/* Backdrop */}
      <div
        onClick={closeInbox}
        style={{
          position: 'fixed', inset: 0, zIndex: 300,
          background: 'rgba(0,0,0,0.28)',
          opacity: inboxOpen ? 1 : 0,
          pointerEvents: inboxOpen ? 'auto' : 'none',
          transition: 'opacity 0.16s ease',
        }}
      />
      <div
        style={{
          position: 'fixed', top: 0, right: 0, bottom: 0, width: DRAWER_WIDTH, maxWidth: '92vw',
          zIndex: 301, display: 'flex', flexDirection: 'column',
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
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: `${14 + captionInsetTop()}px 16px 10px` }}>
          <div style={{ fontSize: '0.95rem', fontWeight: 800, color: 'var(--wks-text-primary)', letterSpacing: '-0.01em' }}>Inbox</div>
          {counts.needsYou > 0 ? (
            <span style={{ fontSize: '0.72rem', fontWeight: 700, color: 'var(--wks-warning, #e0a000)' }}>{counts.needsYou} need you</span>
          ) : counts.total > 0 ? (
            <span style={{ fontSize: '0.72rem', fontWeight: 600, color: 'var(--wks-success, #3fb950)' }}>{counts.total} to review</span>
          ) : (
            <span style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>all clear</span>
          )}
          <div style={{ flex: 1 }} />
          <button onClick={closeInbox} title="Close (Esc)" style={closeBtn}>✕</button>
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
                  flex: 1, padding: '4px 6px', borderRadius: 6, cursor: 'pointer',
                  fontSize: '0.68rem', fontWeight: 600, fontFamily: 'inherit',
                  display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
                  border: active ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border-input)',
                  background: active ? 'var(--wks-accent-bg)' : 'transparent',
                  color: active ? 'var(--wks-accent-text, var(--wks-text-primary))' : 'var(--wks-text-tertiary)',
                }}
              >
                {t.label}
                {t.count > 0 && (
                  <span style={{ fontSize: '0.6rem', fontWeight: 700, opacity: 0.8 }}>{t.count}</span>
                )}
              </button>
            );
          })}
        </div>

        {/* Hint strip + bulk actions */}
        <div style={{ padding: '0 16px 10px', fontSize: '0.62rem', color: 'var(--wks-text-faint)', display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
          <Hint k="j/k" t="move" /><Hint k="y/n" t="approve" /><Hint k="1-9" t="answer" /><Hint k="o" t="open" /><Hint k="e" t="dismiss" /><Hint k="s" t="snooze" />
          {reviewedInFeed.length > 1 && (
            <button
              onClick={clearReviewed}
              title="Dismiss every reviewed (non-blocking) item shown (E)"
              style={{
                marginLeft: 'auto', fontSize: '0.62rem', fontWeight: 600, fontFamily: 'inherit', cursor: 'pointer',
                border: '1px solid var(--wks-border-input)', borderRadius: 6, padding: '2px 9px',
                background: 'transparent', color: 'var(--wks-text-secondary)', whiteSpace: 'nowrap',
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
              <div style={{ display: 'flex', justifyContent: 'center', marginBottom: 8, color: 'var(--wks-success, #3fb950)' }}>
                <CheckCircle2 size={30} strokeWidth={1.75} />
              </div>
              <div style={{ fontSize: '0.82rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}>Inbox zero</div>
              <div style={{ fontSize: '0.72rem', marginTop: 4, lineHeight: 1.5, maxWidth: 240, marginInline: 'auto' }}>
                No agent needs you right now. Approvals, questions, and finished runs will surface here.
              </div>
            </div>
          ) : (
            <div style={{ position: 'relative', width: '100%', height: virtualizer.getTotalSize() }}>
              {virtualizer.getVirtualItems().map((v) => {
                const it = feed[v.index];
                return (
                  <div
                    key={it.signature}
                    data-index={v.index}
                    ref={virtualizer.measureElement}
                    style={{ position: 'absolute', top: 0, left: 0, width: '100%', transform: `translateY(${v.start}px)`, paddingBottom: 12 }}
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
  width: 26, height: 26, display: 'flex', alignItems: 'center', justifyContent: 'center',
  border: '1px solid var(--wks-glass-border)', borderRadius: 6, cursor: 'pointer',
  background: 'var(--wks-bg-surface)', color: 'var(--wks-text-secondary)', fontSize: '0.8rem', lineHeight: 1,
};

const Hint: React.FC<{ k: string; t: string }> = ({ k, t }) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
    <kbd style={{ fontSize: '0.58rem', color: 'var(--wks-text-secondary)', border: '1px solid var(--wks-glass-border)', borderRadius: 3, padding: '0 4px', fontFamily: 'monospace' }}>{k}</kbd>
    <span>{t}</span>
  </span>
);

export default InboxDrawer;
