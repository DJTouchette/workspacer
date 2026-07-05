import React, { useLayoutEffect, useRef, useState } from 'react';
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
  /** Display name of the agent backend (Claude / Codex / …) for placeholders. */
  agentName?: string;
  /** Session controls (model / effort / permission) rendered inside the
   *  panel's bottom row, next to the attach button. */
  controls?: React.ReactNode;
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
}) => {
  const fallbackRef = useRef<HTMLTextAreaElement>(null);
  const taRef = inputRef ?? fallbackRef;
  const [focused, setFocused] = useState(false);
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
            display: 'flex',
            flexDirection: 'column',
            borderRadius: 16,
            border: `1px solid ${borderColor}`,
            background: 'var(--wks-bg-raised)',
            boxShadow: '0 4px 24px rgba(0, 0, 0, 0.18)',
            transition: 'border-color 0.15s',
          }}
        >
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
                : `Message ${agentName}… (Shift+Enter for newline)`
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
              if (e.key === 'Enter' && !e.shiftKey && !ime) {
                e.preventDefault();
                onSend();
              }
            }}
            style={{
              fontSize: '0.82rem',
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
                  backgroundColor: canSend
                    ? 'var(--wks-accent)'
                    : 'var(--wks-bg-elevated, rgba(255,255,255,0.06))',
                  color: canSend ? '#fff' : colors.mutedDim,
                  cursor: canSend ? 'pointer' : 'default',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  fontSize: '0.9rem',
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
        </div>
      </div>
    </div>
  );
};
