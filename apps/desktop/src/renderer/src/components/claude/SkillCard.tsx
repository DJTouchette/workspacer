import React from 'react';
import { Sparkles } from 'lucide-react';
import type { ToolCall } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { useSkillInfo } from '../../contexts/SkillInventoryContext';
import { FileLink } from './FileLink';

/** The origin claudemon stamps on a skill with no file behind it. Mirrors
 *  `BUILTIN_SOURCE` in claude_stream.rs and ContextPane. */
const BUILTIN_SOURCE = 'built-in';

/** The tool name Claude Code uses to invoke a skill. */
export const SKILL_TOOL = 'Skill';

/** Whether this call is a skill invocation (so the work log renders a card
 *  instead of an anonymous tool row). */
export function isSkillCall(tc: ToolCall): boolean {
  return tc.name === SKILL_TOOL;
}

/** The skill's name out of the call input. Claude Code sends `skill`; the other
 *  keys are what claudemon's transcript summarizer already accepts, kept in
 *  agreement with `summarize_tool_input` in transcript.rs. */
export function skillCallName(tc: ToolCall): string {
  const i = tc.input ?? {};
  for (const key of ['skill', 'name', 'command']) {
    const v = i[key];
    if (typeof v === 'string' && v.trim()) return v.trim();
  }
  return '';
}

/** Arguments passed alongside the skill name, when there are any. */
function skillCallArgs(tc: ToolCall): string {
  const v = (tc.input ?? {}).args;
  return typeof v === 'string' ? v.trim() : '';
}

/**
 * One skill invocation in the work log. A skill changes how the agent works for
 * the rest of the turn, so the row says WHICH skill, what it is for, and where
 * it came from — the same three facts the Context pane shows, resolved from the
 * same session inventory rather than a second lookup.
 */
const SkillCardInner: React.FC<{ tc: ToolCall; cwd?: string }> = ({ tc, cwd }) => {
  const name = skillCallName(tc);
  const args = skillCallArgs(tc);
  const info = useSkillInfo(name);
  const failed = tc.status === 'failed';

  return (
    <div style={{ padding: '3px 0' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: '0.75rem' }}>
        <span
          aria-hidden
          style={{
            color: failed ? colors.error : colors.purple,
            display: 'inline-flex',
            flexShrink: 0,
          }}
        >
          <Sparkles size={12} />
        </span>
        <span
          style={{
            color: colors.purple,
            fontFamily: 'var(--claude-mono-font, monospace)',
            fontSize: '0.72rem',
          }}
        >
          {/* The file, when there is one, is the thing a user wants to open
              after seeing a skill fire — it is the instructions that just
              changed the agent's behaviour. */}
          Skill(
          {info?.path ? (
            <FileLink path={info.path} cwd={cwd} style={{ color: colors.purple }}>
              {name || 'skill'}
            </FileLink>
          ) : (
            name || 'skill'
          )}
          )
        </span>
        {info?.source && (
          <span
            title={
              info.source === BUILTIN_SOURCE
                ? 'Shipped inside the Claude Code binary — no file on disk'
                : `Loaded from ${info.source}`
            }
            style={{
              fontSize: '0.62rem',
              color: colors.mutedDim,
              border: `1px solid ${
                info.source === BUILTIN_SOURCE ? 'transparent' : colors.borderSubtle
              }`,
              background: info.source === BUILTIN_SOURCE ? 'rgba(255,255,255,0.04)' : 'transparent',
              borderRadius: 'var(--wks-radius-pill)',
              padding: '0 5px',
              lineHeight: 1.6,
              flexShrink: 0,
            }}
          >
            {info.source}
          </span>
        )}
      </div>
      {args && (
        <div
          style={{
            paddingLeft: 18,
            fontSize: '0.7rem',
            color: colors.muted,
            fontFamily: 'var(--claude-mono-font, monospace)',
            overflowWrap: 'anywhere',
          }}
        >
          {args}
        </div>
      )}
      {info?.description && (
        <div
          title={info.description}
          style={{
            paddingLeft: 18,
            fontSize: '0.69rem',
            lineHeight: 1.45,
            color: colors.mutedDim,
            display: '-webkit-box',
            WebkitLineClamp: 2,
            WebkitBoxOrient: 'vertical',
            overflow: 'hidden',
          }}
        >
          {info.description}
        </div>
      )}
    </div>
  );
};

export const SkillCard = React.memo(SkillCardInner);
