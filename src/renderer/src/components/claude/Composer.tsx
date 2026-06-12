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
}) => {
  const fallbackRef = useRef<HTMLTextAreaElement>(null);
  const taRef = inputRef ?? fallbackRef;
  const canSend = value.trim().length > 0 || attachedFiles.length > 0;

  // Auto-grow: reset to auto then clamp to scrollHeight so shrinking works too.
  useLayoutEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = 'auto';
    const next = Math.min(ta.scrollHeight, MAX_COMPOSER_HEIGHT);
    ta.style.height = `${next}px`;
    ta.style.overflowY = ta.scrollHeight > MAX_COMPOSER_HEIGHT ? 'auto' : 'hidden';
  }, [value, taRef]);

  return (
    <div style={{
      borderTop: `1px solid ${colors.border}`,
      padding: '8px 16px 10px 16px',
      flexShrink: 0,
      opacity: dimmed ? 0.55 : 1,
      transition: 'opacity 0.15s',
    }}>
      <div style={{ maxWidth: 720, margin: '0 auto' }}>
        <FileChips files={attachedFiles} onRemove={onRemoveFile} />
        <div style={{
          display: 'flex',
          alignItems: 'flex-end',
          gap: 6,
          padding: '6px 6px 6px 10px',
          borderRadius: 14,
          border: `1px solid ${attachedFiles.length > 0 ? colors.accent : colors.border}`,
          backgroundColor: 'rgba(255, 255, 255, 0.03)',
          transition: 'border-color 0.15s',
        }}>
          <button
            onClick={onPickFiles}
            title="Attach files"
            style={{
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
              marginBottom: 2,
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
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                onSend();
              }
            }}
            style={{
              flex: 1,
              fontSize: '0.8rem',
              padding: '4px 0',
              border: 'none',
              backgroundColor: 'transparent',
              color: colors.text,
              outline: 'none',
              fontFamily: 'inherit',
              lineHeight: 1.5,
              resize: 'none',
              maxHeight: MAX_COMPOSER_HEIGHT,
            }}
          />
          <button
            onClick={onSend}
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
        </div>
      </div>
    </div>
  );
};
