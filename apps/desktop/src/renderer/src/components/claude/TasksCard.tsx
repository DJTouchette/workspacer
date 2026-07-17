import React, { useState } from 'react';
import type { SessionPlan } from '../../types/claudeSession';
import { claudeColors as colors } from '../claude-shared';
import { AgentSpinner } from './WorkflowAgentRow';

/**
 * The agent's task list / plan as a persistent card pinned above the composer
 * — the tasks counterpart of the workflow/subagent cards. Renders whatever
 * feeds `session.plan` (Claude's TaskCreate/TaskUpdate and TodoWrite, Codex's
 * plan tool), view-only. Shares the composer's centered 1040px column and
 * raised chrome so the two read as one floating cluster.
 *
 * A plan can go stale (the agent abandons its list mid-session), so the card
 * is dismissible: × hides it, and the host re-shows it only when the plan
 * actually changes again (signature-based, see `planSignature`).
 */
export const TasksCard: React.FC<{
  plan: SessionPlan;
  onDismiss: () => void;
}> = ({ plan, onDismiss }) => {
  const done = plan.steps.filter((s) => s.status === 'completed').length;
  const total = plan.steps.length;
  const active = plan.steps.find((s) => s.status === 'in_progress');
  const allDone = total > 0 && done === total;
  // A finished list opens nothing new — start it collapsed; a live one opens.
  const [expanded, setExpanded] = useState(!allDone);

  return (
    <div style={{ padding: '0 16px 6px', flexShrink: 0 }}>
      <div style={{ maxWidth: 1040, margin: '0 auto' }}>
        <div
          style={{
            border: `1px solid ${colors.borderSubtle}`,
            borderRadius: 'var(--wks-radius-lg)',
            background: 'var(--wks-bg-raised)',
            boxShadow: '0 2px 12px rgba(0, 0, 0, 0.12)',
            overflow: 'hidden',
          }}
        >
          <div
            onClick={() => setExpanded((e) => !e)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: '6px 12px',
              cursor: 'pointer',
              userSelect: 'none',
              fontSize: '0.72rem',
            }}
          >
            <span style={{ color: colors.mutedDim, fontSize: '0.64rem', flexShrink: 0, width: 8 }}>
              {expanded ? '▾' : '▸'}
            </span>
            {active ? (
              <AgentSpinner color="var(--wks-accent)" />
            ) : (
              <span
                style={{
                  color: allDone ? colors.success : colors.muted,
                  fontSize: '0.7rem',
                  width: 12,
                  textAlign: 'center',
                  flexShrink: 0,
                }}
              >
                {allDone ? '✓' : '☰'}
              </span>
            )}
            <span style={{ color: colors.textBright, fontWeight: 600, flexShrink: 0 }}>Tasks</span>
            <span
              style={{
                color: colors.muted,
                fontVariantNumeric: 'tabular-nums',
                flexShrink: 0,
              }}
            >
              {done}/{total}
            </span>
            {/* Thin progress bar */}
            <div
              style={{
                flex: '0 1 140px',
                height: 3,
                borderRadius: 2,
                background: 'var(--wks-bg-hover)',
                overflow: 'hidden',
                minWidth: 40,
              }}
            >
              <div
                style={{
                  width: `${total ? Math.round((done / total) * 100) : 0}%`,
                  height: '100%',
                  borderRadius: 2,
                  background: allDone ? colors.success : 'var(--wks-accent)',
                  transition: 'width 300ms ease',
                }}
              />
            </div>
            {/* Collapsed, the header carries the live "doing now" line so the
                card stays useful without taking the space of the full list. */}
            {!expanded && active && (
              <span
                style={{
                  color: colors.muted,
                  minWidth: 0,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {active.activeForm ?? active.content}
              </span>
            )}
            <div style={{ flex: 1 }} />
            <button
              onClick={(e) => {
                e.stopPropagation();
                onDismiss();
              }}
              title="Dismiss (reappears when the tasks change)"
              aria-label="Dismiss tasks"
              style={{
                background: 'none',
                border: 'none',
                padding: '0 2px',
                cursor: 'pointer',
                color: colors.mutedDim,
                fontSize: '0.85rem',
                lineHeight: 1,
                flexShrink: 0,
              }}
            >
              ×
            </button>
          </div>
          {expanded && (
            <div style={{ padding: '0 12px 8px 28px', maxHeight: 200, overflowY: 'auto' }}>
              {plan.steps.map((s, i) => {
                const running = s.status === 'in_progress';
                const completed = s.status === 'completed';
                return (
                  <div
                    key={i}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 8,
                      padding: '2px 0',
                      fontSize: '0.72rem',
                      lineHeight: 1.5,
                    }}
                  >
                    <span
                      style={{
                        width: 12,
                        textAlign: 'center',
                        flexShrink: 0,
                        display: 'inline-flex',
                        justifyContent: 'center',
                      }}
                    >
                      {running ? (
                        <AgentSpinner color="var(--wks-accent)" />
                      ) : (
                        <span
                          style={{
                            color: completed ? colors.success : colors.mutedDim,
                            fontSize: '0.69rem',
                          }}
                        >
                          {completed ? '✓' : '○'}
                        </span>
                      )}
                    </span>
                    <span
                      style={{
                        color: completed
                          ? colors.mutedDim
                          : running
                            ? colors.textBright
                            : colors.text,
                        textDecoration: completed ? 'line-through' : 'none',
                        minWidth: 0,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                      title={s.content}
                    >
                      {running && s.activeForm ? s.activeForm : s.content}
                    </span>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

/** Stable fingerprint of a plan's visible state — the dismiss key: dismissing
 *  hides the card for THIS state, and any real change (status flip, step
 *  added/renamed) produces a new signature that brings it back. */
export function planSignature(plan: SessionPlan | undefined): string {
  if (!plan || plan.steps.length === 0) return '';
  return plan.steps.map((s) => `${s.status}:${s.content}`).join(' ');
}
