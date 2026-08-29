import React, { useState } from 'react';
import { X } from 'lucide-react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import { WIDGET_BOARD_WIDTH } from '../../types/widget';
import { claudeColors as colors } from '../claude-shared';
import { usePluginsContext } from '../../contexts/PluginsContext';
import { InspectorCard } from './InspectorCard';
import { WidgetBoard } from '../widgets/WidgetBoard';

/**
 * Right-hand inspector for the Claude pane GUI view: a persistent home for the
 * session's plan, files, agents, and usage — so workflow/agent state doesn't
 * live only in the scrollback. It is the docked chrome (fixed width + left
 * border + a close control) around the shared {@link InspectorCard}, which owns
 * the tab strip and all tab bodies; the same card also powers the standalone
 * inspector pane, the Fleet-Deck card expansion and the sidebar hover peek.
 *
 * The rail hosts two things at two different scopes, switched by the segmented
 * control in its header:
 *
 *   Session — the InspectorCard, everything about the agent you're piloting.
 *   Project — the {@link WidgetBoard}, everything about the *directory* that
 *             agent is working in, keyed by cwd and shared by every agent there.
 *
 * The board is a sibling of the card, never a sixth RailTab: InspectorCard
 * guarantees it renders purely from the snapshot it's passed and never fetches,
 * and a project-scoped grid that polls git does not belong under that promise.
 */
export const InspectorRail: React.FC<{
  session: ClaudeSessionSnapshot | null;
  /** The bound session id — carried so the card can still show the history DB's
   *  recorded cost/tokens on a cold start, when `session` is null. */
  sessionId?: string;
  onClose: () => void;
  /**
   * The directory the board belongs to — the pane's `effectiveCwd`, which is
   * the live worktree when the session reports one and the spawn dir otherwise.
   * Passed in rather than derived from the snapshot so the board is usable
   * before a session exists (and keeps working if one never attaches).
   */
  cwd?: string;
}> = ({ session, sessionId, onClose, cwd: cwdProp }) => {
  const [view, setView] = useState<'session' | 'project'>('session');
  const { widgets } = usePluginsContext();
  const cwd = cwdProp || session?.liveCwd || session?.cwd || '';

  const closeButton = (
    <button
      onClick={onClose}
      title="Hide inspector"
      style={{
        border: 'none',
        background: 'transparent',
        color: colors.mutedDim,
        cursor: 'pointer',
        padding: '2px 6px',
        display: 'flex',
        alignItems: 'center',
      }}
    >
      <X size={13} strokeWidth={2} />
    </button>
  );

  return (
    <div
      style={{
        // Derived from the widget cell size rather than hardcoded, so a small
        // widget lands square at ~148px — within a few points of the ~155pt
        // square an iPhone home screen uses. See types/widget.ts.
        width: WIDGET_BOARD_WIDTH,
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        borderLeft: `1px solid ${colors.border}`,
        backgroundColor: 'rgba(255,255,255,0.012)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 4,
          padding: '6px 8px 0',
          flexShrink: 0,
        }}
      >
        {(['session', 'project'] as const).map((v) => (
          <button
            key={v}
            onClick={() => setView(v)}
            style={{
              border: 'none',
              background: view === v ? 'rgba(255,255,255,0.07)' : 'transparent',
              color: view === v ? colors.text : colors.mutedDim,
              borderRadius: 6,
              fontSize: 11,
              padding: '3px 9px',
              cursor: 'pointer',
              textTransform: 'capitalize',
            }}
          >
            {v}
          </button>
        ))}
        <div style={{ marginLeft: 'auto' }}>{closeButton}</div>
      </div>

      {/* Switching unmounts the other view. That is deliberate for the board:
          tearing down its plugin guests is how they stay affordable, and the
          hibernation sweep never sees them (it only walks panes inside tabs). */}
      {view === 'session' ? (
        <InspectorCard snapshot={session} sessionId={sessionId} />
      ) : (
        <WidgetBoard cwd={cwd} snapshot={session} available={widgets} />
      )}
    </div>
  );
};
