import React, { useEffect, useRef, useState } from 'react';
import { MoreHorizontal } from 'lucide-react';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { ContextMenu } from './ContextMenu';
import { TerminateAgentButton } from './TerminateAgentButton';

/** One menu for the deck: opening it never changes the selected agent or chat. */
export function useFleetAgentMenu({
  agents,
  snapshotBySession,
  onTerminate,
  disabledReason,
}: {
  agents: AgentWorkspace[];
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
  onTerminate?: (id: string) => Promise<void>;
  disabledReason?: string;
}) {
  const [target, setTarget] = useState<{ id: string; x: number; y: number } | null>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const content = useRef<HTMLDivElement>(null);
  const agent = agents.find((a) => a.id === target?.id);
  const close = () => {
    setTarget(null);
    previousFocus.current?.focus({ preventScroll: true });
  };
  useEffect(() => {
    if (!target) return;
    // ContextMenu hides its first render until its layout measurement commits.
    const frame = requestAnimationFrame(() => {
      (
        content.current?.querySelector<HTMLElement>('button:not(:disabled)') ?? content.current
      )?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [target]);
  useEffect(() => {
    if (target && !agent) setTarget(null);
  }, [target, agent]);

  const open = (id: string, x: number, y: number) => {
    if (!target) previousFocus.current = document.activeElement as HTMLElement | null;
    setTarget({ id, x, y });
  };
  const handlers = (id: string) => ({
    onContextMenu: (e: React.MouseEvent<HTMLElement>) => {
      // Preserve native editing menus inside the card's composer.
      if ((e.target as Element).closest('input, textarea, [contenteditable="true"]')) return;
      e.preventDefault();
      e.stopPropagation();
      const rect = e.currentTarget.getBoundingClientRect();
      open(id, e.clientX || rect.left, e.clientY || rect.bottom);
    },
    onKeyDown: (e: React.KeyboardEvent<HTMLElement>) => {
      if (e.key !== 'ContextMenu' && !(e.shiftKey && e.key === 'F10')) return;
      if ((e.target as Element).closest('input, textarea, [contenteditable="true"]')) return;
      e.preventDefault();
      e.stopPropagation();
      const rect = e.currentTarget.getBoundingClientRect();
      open(id, rect.left, rect.bottom);
    },
  });
  const overflow = (a: AgentWorkspace) => (
    <button
      type="button"
      data-fleet-action="menu"
      title={`Actions for ${a.name}`}
      aria-label={`Actions for ${a.name}`}
      aria-haspopup="menu"
      aria-expanded={target?.id === a.id}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        const rect = e.currentTarget.getBoundingClientRect();
        open(a.id, rect.left, rect.bottom);
      }}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        flexShrink: 0,
        width: 28,
        height: 28,
        padding: 4,
        border: 'none',
        borderRadius: 'var(--wks-radius-md)',
        background: 'transparent',
        color: 'var(--wks-text-secondary)',
        cursor: 'pointer',
      }}
    >
      <MoreHorizontal size={14} />
    </button>
  );
  const menu =
    target && agent ? (
      <ContextMenu x={target.x} y={target.y} onClose={close}>
        <div
          key={agent.id}
          ref={content}
          data-fleet-action="menu"
          tabIndex={-1}
          aria-label={`Actions for ${agent.name}`}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === 'Tab') {
              e.preventDefault();
              close();
            } else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) {
              e.preventDefault();
              const buttons = [
                ...e.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'),
              ];
              const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
              const next =
                e.key === 'Home'
                  ? 0
                  : e.key === 'End'
                    ? buttons.length - 1
                    : (current + (e.key === 'ArrowUp' ? -1 : 1) + buttons.length) % buttons.length;
              buttons[next]?.focus();
            }
          }}
        >
          <TerminateAgentButton
            menu
            agent={agent}
            snapshot={agent.sessionId ? snapshotBySession[agent.sessionId] : undefined}
            onTerminate={onTerminate}
            disabledReason={disabledReason}
            onClose={close}
          />
        </div>
      </ContextMenu>
    ) : null;
  return { handlers, overflow, menu };
}
