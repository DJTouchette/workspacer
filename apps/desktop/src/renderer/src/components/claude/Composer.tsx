import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowUp } from 'lucide-react';
import { claudeColors as colors } from '../claude-shared';
import { FileChips } from './FileChips';
import type { AttachedFile } from './fileAttachment';
import { filterSlashItems, type SlashItem } from '../../lib/slashItems';

/** Cap auto-grow at roughly 8 lines; beyond that the textarea scrolls. */
const MAX_COMPOSER_HEIGHT = 168;

export interface ComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  onPaste?: (e: React.ClipboardEvent) => void;
  onPickFiles: () => void;
  attachedFiles: AttachedFile[];
  onRemoveFile: (idx: number) => void;
  /** Visually recede while a needs-you prompt is docked above. */
  dimmed?: boolean;
  inputRef?: React.RefObject<HTMLTextAreaElement>;
  /** Show the send (↑) button. When false, Enter still sends. Defaults to true. */
  showSendButton?: boolean;
  /** Display name of the agent backend (Claude / Codex / …) for placeholders. */
  agentName?: string;
  /** Session controls (model / effort / permission) rendered inside the
   *  panel's bottom row, next to the attach button. */
  controls?: React.ReactNode;
  /** Skill / prompt candidates for the "/" picker. When present and the input
   *  is a bare "/token", a filtered picker opens above the composer (arrow keys
   *  move, Enter/Tab pick, Escape dismisses). Empty/undefined disables it. */
  slashItems?: SlashItem[];
  /** Invoked with the chosen item's id; the parent resolves + inserts it. */
  onSlashPick?: (id: string) => void;
}

/**
 * Multi-line auto-growing message composer. Enter sends, Shift+Enter inserts a
 * newline.
 *
 * Visually a single floating panel over the conversation background (no
 * full-width strip behind it): rounded border, flat raised surface, the
 * textarea borderless on top and a controls row along the bottom edge —
 * attach + session pills on the left, send on the right.
 */
export const Composer: React.FC<ComposerProps> = ({
  value,
  onChange,
  onSend,
  onPaste,
  onPickFiles,
  attachedFiles,
  onRemoveFile,
  dimmed,
  inputRef,
  showSendButton = true,
  agentName = 'Claude',
  controls,
  slashItems,
  onSlashPick,
}) => {
  const fallbackRef = useRef<HTMLTextAreaElement>(null);
  const taRef = inputRef ?? fallbackRef;
  const [focused, setFocused] = useState(false);
  const canSend = value.trim().length > 0 || attachedFiles.length > 0;

  // ── "/" command picker ──────────────────────────────────────────────────
  // Active only while the whole input is a bare "/token" (no space yet) — once
  // the user types a space they're adding arguments, so the picker steps aside.
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const slashMatch = slashItems && slashItems.length > 0 ? /^\/(\S*)$/.exec(value) : null;
  const slashQuery = slashMatch ? slashMatch[1] : null;
  const slashResults = slashQuery !== null ? filterSlashItems(slashItems!, slashQuery) : [];
  const slashOpen = slashQuery !== null && !slashDismissed && slashResults.length > 0;
  const slashSel = Math.min(slashIndex, Math.max(0, slashResults.length - 1));

  // Reset the highlight as the query changes; drop the manual dismiss once the
  // input is no longer a "/token" so the picker can re-open on the next "/".
  useEffect(() => {
    setSlashIndex(0);
  }, [slashQuery]);
  useEffect(() => {
    if (slashQuery === null) setSlashDismissed(false);
  }, [slashQuery]);

  const pickSlash = (item: SlashItem | undefined) => {
    if (!item) return;
    setSlashDismissed(false);
    onSlashPick?.(item.id);
  };

  // Auto-grow: reset to auto then clamp to scrollHeight so shrinking works too.
  useLayoutEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = 'auto';
    // An empty box is always the natural one-line height — never measure it.
    // A measurement taken while the surrounding layout is still settling (e.g.
    // on session resume, with siblings like the tasks card mounting above)
    // can read a stretched scrollHeight and pin an empty composer tall until
    // the next keystroke re-measures.
    if (value === '') {
      ta.style.overflowY = 'hidden';
      return;
    }
    // While the pane is hidden (display:none ancestor — e.g. before the first
    // GUI switch) scrollHeight reads 0; collapsing the box to 0px there is what
    // left the first message half-clipped. Leave it at the natural one-line
    // height ('auto' + rows=1) and let a later measurement (once visible/typed)
    // size it correctly.
    const sh = ta.scrollHeight;
    if (sh === 0) return;
    const next = Math.min(sh, MAX_COMPOSER_HEIGHT);
    ta.style.height = `${next}px`;
    ta.style.overflowY = sh > MAX_COMPOSER_HEIGHT ? 'auto' : 'hidden';
  }, [value, taRef]);

  const borderColor =
    attachedFiles.length > 0
      ? colors.accent
      : focused
        ? 'var(--wks-border-input)'
        : colors.borderSubtle;

  return (
    <div
      style={{
        padding: '4px 16px 12px',
        flexShrink: 0,
        opacity: dimmed ? 0.55 : 1,
        transition: 'opacity 0.15s',
      }}
    >
      <div style={{ maxWidth: 1040, margin: '0 auto' }}>
        <div
          style={{
            position: 'relative',
            display: 'flex',
            flexDirection: 'column',
            borderRadius: 'var(--wks-radius-lg)',
            border: `1px solid ${borderColor}`,
            background: 'var(--wks-bg-raised)',
            boxShadow: '0 4px 24px rgba(0, 0, 0, 0.18)',
            transition: 'border-color 0.15s',
          }}
        >
          {slashOpen && (
            <div
              role="listbox"
              aria-label="Skills and commands"
              style={{
                position: 'absolute',
                bottom: 'calc(100% + 6px)',
                left: 0,
                right: 0,
                maxHeight: 260,
                overflowY: 'auto',
                padding: 5,
                borderRadius: 'var(--wks-radius-lg)',
                border: '1px solid var(--wks-glass-border)',
                background: 'var(--wks-glass-strong)',
                backdropFilter: 'blur(12px) saturate(160%)',
                WebkitBackdropFilter: 'blur(12px) saturate(160%)',
                boxShadow: '0 12px 34px rgba(0, 0, 0, 0.3)',
                zIndex: 30,
              }}
            >
              {slashResults.map((it, i) => (
                <button
                  key={it.id}
                  role="option"
                  aria-selected={i === slashSel}
                  onMouseEnter={() => setSlashIndex(i)}
                  // pointerdown (not click) + preventDefault so the pick fires
                  // before the textarea blurs — keeping focus and beating any
                  // outside-close — and so a touch tap registers on mobile.
                  onPointerDown={(e) => {
                    e.preventDefault();
                    pickSlash(it);
                  }}
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    alignItems: 'flex-start',
                    gap: 1,
                    width: '100%',
                    textAlign: 'left',
                    border: 'none',
                    borderRadius: 'var(--wks-radius-md)',
                    padding: '6px 10px',
                    cursor: 'pointer',
                    font: 'inherit',
                    background:
                      i === slashSel
                        ? 'color-mix(in srgb, var(--wks-accent) 16%, transparent)'
                        : 'transparent',
                    color: colors.text,
                  }}
                >
                  <span
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 6,
                      fontSize: '0.8rem',
                      fontWeight: 600,
                      maxWidth: '100%',
                    }}
                  >
                    <span style={{ whiteSpace: 'nowrap' }}>
                      <span style={{ opacity: 0.5 }}>/</span>
                      {it.label}
                    </span>
                    {it.kind && (
                      <span
                        style={{
                          fontSize: '0.6rem',
                          fontWeight: 700,
                          letterSpacing: '0.04em',
                          textTransform: 'uppercase',
                          padding: '1px 5px',
                          borderRadius: 'var(--wks-radius-pill)',
                          color: 'var(--wks-text-faint)',
                          border: '1px solid var(--wks-border-subtle)',
                          flexShrink: 0,
                        }}
                      >
                        {it.kind}
                      </span>
                    )}
                  </span>
                  {it.hint && (
                    <span
                      style={{
                        fontSize: '0.7rem',
                        color: colors.muted,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                        maxWidth: '100%',
                      }}
                    >
                      {it.hint}
                    </span>
                  )}
                </button>
              ))}
              <div
                style={{
                  padding: '4px 10px 2px',
                  fontSize: '0.62rem',
                  color: 'var(--wks-text-faint)',
                  fontFamily: 'var(--wks-font-mono)',
                }}
              >
                ↑↓ navigate · enter insert · esc dismiss
              </div>
            </div>
          )}
          {attachedFiles.length > 0 && (
            <div style={{ padding: '10px 14px 0' }}>
              <FileChips files={attachedFiles} onRemove={onRemoveFile} />
            </div>
          )}
          <textarea
            ref={taRef}
            rows={1}
            placeholder={
              attachedFiles.length > 0
                ? `What should ${agentName} do with these files?`
                : `Give ${agentName} something to do…`
            }
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onPaste={onPaste}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            onKeyDown={(e) => {
              // Enter sends; Shift+Enter inserts a newline. Only treat Enter as
              // an IME candidate-commit (not a send) when it's a real
              // composition keystroke — keyCode 229 is the universal
              // "IME is processing this" sentinel. On Linux/Electron (IBus/
              // fcitx) `isComposing` alone spuriously reports true on the first
              // Enter after focus, which swallowed the first message; gating on
              // keyCode 229 too lets that genuine Enter through.
              const ime = e.nativeEvent.isComposing && e.nativeEvent.keyCode === 229;
              // The "/" picker owns the arrow/Enter/Tab/Escape keys while open, so
              // Enter picks an item instead of sending and arrows move the
              // highlight instead of the caret. Runs before the send branch.
              if (slashOpen && !ime) {
                if (e.key === 'ArrowDown') {
                  e.preventDefault();
                  setSlashIndex((i) => (i + 1) % slashResults.length);
                  return;
                }
                if (e.key === 'ArrowUp') {
                  e.preventDefault();
                  setSlashIndex((i) => (i - 1 + slashResults.length) % slashResults.length);
                  return;
                }
                if (e.key === 'Enter' || e.key === 'Tab') {
                  e.preventDefault();
                  pickSlash(slashResults[slashSel]);
                  return;
                }
                if (e.key === 'Escape') {
                  e.preventDefault();
                  setSlashDismissed(true);
                  return;
                }
              }
              if (e.key === 'Enter' && !e.shiftKey && !ime) {
                e.preventDefault();
                onSend();
              }
            }}
            style={{
              fontSize: '0.8rem',
              // border-box so the auto-grow height (set to scrollHeight) matches
              // the text exactly.
              boxSizing: 'border-box',
              padding: '12px 14px 4px',
              border: 'none',
              backgroundColor: 'transparent',
              color: colors.text,
              outline: 'none',
              fontFamily: 'inherit',
              lineHeight: 1.5,
              resize: 'none',
              // Floor at one line (lineHeight + vertical padding, border-box) so
              // the box always shows a full line even if a measurement lands
              // while hidden and reports 0.
              minHeight: 'calc(1.5em + 16px)',
              maxHeight: MAX_COMPOSER_HEIGHT,
            }}
          />
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 4,
              padding: '4px 8px 8px',
              minWidth: 0,
            }}
          >
            <button
              onClick={onPickFiles}
              title="Attach files"
              className="wks-composer-icon-btn"
              style={{
                width: 26,
                height: 26,
                borderRadius: '50%',
                border: 'none',
                backgroundColor: 'transparent',
                color: colors.muted,
                cursor: 'pointer',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: '1.05rem',
                flexShrink: 0,
                padding: 0,
              }}
            >
              +
            </button>
            {controls && (
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 2,
                  minWidth: 0,
                  overflow: 'hidden',
                }}
              >
                {controls}
              </div>
            )}
            <div style={{ flex: 1 }} />
            {showSendButton && (
              <button
                onClick={onSend}
                disabled={!canSend}
                style={{
                  width: 30,
                  height: 30,
                  borderRadius: '50%',
                  border: 'none',
                  backgroundColor: canSend ? 'var(--wks-accent)' : 'var(--wks-bg-elevated)',
                  color: canSend ? '#fff' : colors.mutedDim,
                  cursor: canSend ? 'pointer' : 'default',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  flexShrink: 0,
                  transition: 'background-color 0.15s, color 0.15s',
                }}
                aria-label="Send message"
              >
                <ArrowUp size={15} strokeWidth={2.25} />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
