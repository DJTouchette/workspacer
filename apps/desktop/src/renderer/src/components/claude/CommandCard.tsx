import React, { useMemo, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import type { ConversationTurn } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { parseMarkdownBlocks } from '../markdown';
import { TurnStamp } from './ConversationMessage';

/** Output taller than this starts collapsed behind a "show output" toggle. */
const AUTO_EXPAND_MAX_LINES = 12;

/**
 * A slash-command run in the conversation: `/name args` as a compact
 * right-aligned chip (it's something the user did, like a user bubble), with
 * the command's local output — when it produced any — in a collapsible
 * left-aligned block underneath. Output is markdown-rendered (built-ins like
 * /context emit tables) and pre-stripped of ANSI by the daemon.
 */
const CommandCardInner: React.FC<{ turn: ConversationTurn; showTimestamp?: boolean }> = ({
  turn,
  showTimestamp,
}) => {
  const cmd = turn.command;
  const output = cmd?.output ?? '';
  const outputLines = useMemo(() => (output ? output.split('\n').length : 0), [output]);
  const [open, setOpen] = useState<boolean | null>(null); // null = auto
  const expanded = open ?? outputLines <= AUTO_EXPAND_MAX_LINES;
  const parsedOutput = useMemo(() => (output ? parseMarkdownBlocks(output) : null), [output]);
  if (!cmd) return null;

  return (
    <div style={{ marginBottom: 12, animation: 'claudeFadeIn 0.2s ease-out' }}>
      {/* Invocation chip — user-side, so right-aligned like a user bubble */}
      {cmd.name && (
        <div style={{ display: 'flex', justifyContent: 'flex-end', alignItems: 'center', gap: 8 }}>
          {showTimestamp && <TurnStamp ms={turn.timestamp} />}
          <div
            style={{
              display: 'inline-flex',
              alignItems: 'baseline',
              gap: 7,
              maxWidth: '80%',
              padding: '5px 12px',
              borderRadius: 'var(--wks-radius-pill, 999px)',
              border: `1px solid ${colors.userBubbleBorder}`,
              backgroundColor: colors.userBubble,
              fontFamily: 'var(--claude-mono-font, monospace)',
              fontSize: 'calc(0.74rem * var(--claude-gui-font-scale, 1))',
            }}
          >
            <span style={{ color: colors.accent, fontWeight: 600, whiteSpace: 'nowrap' }}>
              /{cmd.name}
            </span>
            {cmd.args && (
              <span
                style={{
                  color: colors.textBright,
                  wordBreak: 'break-word',
                }}
              >
                {cmd.args}
              </span>
            )}
          </div>
        </div>
      )}

      {/* Local output — assistant-side block under the chip */}
      {output && (
        <div style={{ marginTop: cmd.name ? 8 : 0 }}>
          <button
            onClick={() => setOpen(!expanded)}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              padding: 0,
              border: 'none',
              background: 'transparent',
              cursor: 'pointer',
              fontFamily: 'inherit',
              fontSize: '0.69rem',
              fontWeight: 600,
              color: cmd.outputIsError ? colors.error : colors.muted,
            }}
          >
            <ChevronRight
              size={11}
              aria-hidden
              style={{
                transform: expanded ? 'rotate(90deg)' : 'none',
                transition: 'transform 0.15s',
              }}
            />
            output{outputLines > 1 ? ` · ${outputLines} lines` : ''}
            {cmd.outputIsError ? ' · error' : ''}
          </button>
          {expanded && (
            <div
              style={{
                marginTop: 6,
                padding: '8px 12px',
                borderLeft: `2px solid ${cmd.outputIsError ? colors.error : colors.borderSubtle}`,
                fontSize: 'calc(0.76rem * var(--claude-gui-font-scale, 1))',
                lineHeight: 1.55,
                color: colors.text,
                wordBreak: 'break-word',
              }}
            >
              {parsedOutput}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export const CommandCard = React.memo(CommandCardInner);
