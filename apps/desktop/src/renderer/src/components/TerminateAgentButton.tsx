import React, { useRef, useState } from 'react';
import { Square } from 'lucide-react';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { SmallButton } from './settings/primitives';

export function terminationUnavailable(
  agent: AgentWorkspace,
  snapshot: ClaudeSessionSnapshot | undefined,
  available: boolean,
): string | undefined {
  if (!available) return 'Agent control is unavailable';
  if (snapshot?.hubOffline) return 'Hub is offline — reconnect to terminate this agent';
  if (agent.hub && !snapshot) return 'Remote session authority is unavailable';
  return undefined;
}

/** Deliberately separate from card activation. No shared dialog primitive exists. */
export function TerminateAgentButton({
  agent,
  snapshot,
  onTerminate,
  disabledReason,
}: {
  agent: AgentWorkspace;
  snapshot?: ClaudeSessionSnapshot;
  onTerminate?: (id: string) => Promise<void>;
  disabledReason?: string;
}) {
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const pending = useRef(false);
  const reason = disabledReason ?? terminationUnavailable(agent, snapshot, !!onTerminate);
  const confirm = async () => {
    if (pending.current || reason || !onTerminate) return;
    pending.current = true;
    setBusy(true);
    setError('');
    try {
      await onTerminate(agent.id);
      setArmed(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      pending.current = false;
      setBusy(false);
    }
  };
  return (
    <span
      title={reason ?? `Terminate ${agent.name}`}
      data-fleet-action="terminate"
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => e.stopPropagation()}
      style={{
        display: 'inline-flex',
        flexWrap: 'wrap',
        alignItems: 'center',
        gap: 6,
        fontSize: '0.72rem',
      }}
    >
      {armed ? (
        <>
          <span>Terminate {agent.name}?</span>
          <SmallButton
            danger
            disabled={busy || !!reason}
            onClick={() => void confirm()}
            label={busy ? 'Terminating…' : 'Confirm terminate'}
          />
          <SmallButton
            disabled={busy}
            onClick={() => {
              setArmed(false);
              setError('');
            }}
            label="Cancel"
          />
        </>
      ) : (
        <SmallButton
          danger
          disabled={!!reason}
          onClick={() => setArmed(true)}
          label={
            <>
              <Square size={12} /> Terminate
            </>
          }
        />
      )}
      {reason && <span style={{ color: 'var(--wks-text-disabled)' }}>{reason}</span>}
      {error && (
        <span role="alert" style={{ color: 'var(--wks-error)' }}>
          {error}
        </span>
      )}
    </span>
  );
}
