import React, { useLayoutEffect, useRef } from 'react';
import { claudeColors as colors } from '../claude-shared';
import { FileChips } from './FileChips';
import type { AttachedFile } from './fileAttachment';

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
}

/**
 * Multi-line auto-growing message composer. Enter sends, Shift+Enter inserts
 * a newline. Replaces the old single-line <input> so pasted/typed multi-line
 * prompts are visible and editable before sending.
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
}) => {
  const fallbackRef = useRef<HTMLTextAreaElement>(null);
  const taRef = inputRef ?? fallbackRef;
  const canSend = value.trim().length > 0 || attachedFiles.length > 0;

  // Auto-grow: reset to auto then clamp to scrollHeight so shrinking works too.
  useLayoutEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = 'auto';
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

  return (
    <div style={{
      borderTop: `1px solid ${colors.border}`,
      background: 'var(--wks-bg-raised)',
      padding: '10px 16px 9px 16px',
      flexShrink: 0,
      opacity: dimmed ? 0.55 : 1,
      transition: 'opacity 0.15s',
    }}>
      <div style={{ maxWidth: 1040, margin: '0 auto' }}>
        <FileChips files={attachedFiles} onRemove={onRemoveFile} />
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '6px 6px 6px 10px',
          borderRadius: 13,
          border: `1px solid ${attachedFiles.length > 0 ? colors.accent : 'var(--wks-border-input)'}`,
          backgroundColor: 'var(--wks-bg-input)',
          boxShadow: 'inset 0 1px 0 var(--wks-glass-highlight)',
          transition: 'border-color 0.15s',
        }}>
          <span aria-hidden style={{
            alignSelf: 'center', flexShrink: 0,
            fontFamily: 'var(--wks-font-mono)', fontSize: '0.95rem', fontWeight: 700,
            color: colors.accent, lineHeight: 1,
          }}>{'›'}</span>
          <button
            onClick={onPickFiles}
            title="Attach files"
            style={{
              alignSelf: 'center',
              width: 24,
              height: 24,
              borderRadius: '50%',
              border: 'none',
              backgroundColor: 'transparent',
              color: colors.muted,
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              fontSize: '1rem',
              flexShrink: 0,
              padding: 0,
            }}
          >
            +
          </button>
          <textarea
            ref={taRef}
            rows={1}
            placeholder={attachedFiles.length > 0 ? 'What should Claude do with these files?' : 'Message Claude... (Shift+Enter for newline)'}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onPaste={onPaste}
            onKeyDown={(e) => {
              // Enter sends; Shift+Enter inserts a newline. Only treat Enter as
              // an IME candidate-commit (not a send) when it's a real
              // composition keystroke — keyCode 229 is the universal
              // "IME is processing this" sentinel. On Linux/Electron (IBus/
              // fcitx) `isComposing` alone spuriously reports true on the first
              // Enter after focus, which swallowed the first message; gating on
              // keyCode 229 too lets that genuine Enter through.
              const ime = e.nativeEvent.isComposing && e.nativeEvent.keyCode === 229;
              if (e.key === 'Enter' && !e.shiftKey && !ime) {
                e.preventDefault();
                onSend();
              }
            }}
            style={{
              flex: 1,
              fontSize: '0.8rem',
              // border-box so the auto-grow height (set to scrollHeight) matches
              // the text exactly; otherwise the padding inflates the box and the
              // top-aligned text rides above the centered buttons.
              boxSizing: 'border-box',
              padding: '4px 0',
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
              minHeight: 'calc(1.5em + 8px)',
              maxHeight: MAX_COMPOSER_HEIGHT,
            }}
          />
          {showSendButton && (
            <button
              onClick={onSend}
              disabled={!canSend}
              style={{
                width: 28,
                height: 28,
                borderRadius: '50%',
                border: 'none',
                backgroundColor: canSend ? colors.accent : 'rgba(255,255,255,0.06)',
                color: canSend ? '#0d0d10' : colors.mutedDim,
                cursor: canSend ? 'pointer' : 'default',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: '0.85rem',
                fontWeight: 700,
                flexShrink: 0,
                transition: 'background-color 0.15s, color 0.15s',
              }}
              aria-label="Send message"
            >
              {'↑'}
            </button>
          )}
        </div>
        {/* Key hints (mockup) — subtle, and the discoverable cue for Enter-to-send
            when the send button is hidden. */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10, marginTop: 6, padding: '0 3px',
          fontFamily: 'var(--wks-font-mono, monospace)', fontSize: '0.6rem', color: colors.mutedDim,
        }}>
          <span><span style={{ color: colors.muted }}>⏎</span> send</span>
          <span><span style={{ color: colors.muted }}>⇧⏎</span> newline</span>
        </div>
      </div>
    </div>
  );
};
