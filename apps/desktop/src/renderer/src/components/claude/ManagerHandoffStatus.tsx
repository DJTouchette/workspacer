import { requestSessionWatch } from '../../lib/watchBus';
import type { AgentProvider } from '../../types/pane';
import React from 'react';
import type { ManagerReplacementView } from '../../../../main/shared/managerReplacement';
import { managerReplacementRequest } from '../../lib/managerReplacement';
import { SmallButton } from '../settings/primitives';

const phaseLabel: Record<ManagerReplacementView['phase'], string> = {
  preparing: 'Checkpointing and writing handoff…',
  spawning: 'Starting fresh manager…',
  transferring: 'Transferring workers and tasks…',
  binding: 'Attaching the fresh manager to this pane…',
  activating: 'Starting the successor…',
  complete: 'Manager handoff complete',
  failed: 'Handoff failed — old manager retained',
  cancelled: 'Handoff cancelled — old manager retained',
  'recovery-required': 'Manager handoff needs attention',
};
export function ManagerHandoffStatus({
  operation,
  error,
  provider,
  cwd,
}: {
  operation?: ManagerReplacementView;
  error?: string;
  provider?: AgentProvider;
  cwd?: string;
}) {
  if (!operation && !error) return null;
  const request = (action: 'cancel' | 'reconcile') =>
    operation && void managerReplacementRequest({ action, operationId: operation.operationId });
  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        width: '100%',
        maxWidth: 'var(--wks-chat-width)',
        margin: '0 auto',
        boxSizing: 'border-box',
        fontSize: '0.72rem',
        padding: '6px 12px',
        color:
          operation?.phase === 'recovery-required' || error
            ? 'var(--wks-warning)'
            : 'var(--wks-text-secondary)',
      }}
    >
      {operation && <strong>{phaseLabel[operation.phase]}</strong>}
      {(error || operation?.error) && <p>{error || operation?.error}</p>}
      {operation && (
        <>
          <div>
            {operation.workerIds.length} workers · {operation.taskIds.length} tasks ·{' '}
            {operation.committed ? 'Workers and tasks transferred' : 'No ownership transfer yet'}
          </div>
          {['preparing', 'spawning'].includes(operation.phase) && (
            <SmallButton onClick={() => request('cancel')} label="Cancel handoff" />
          )}
          {operation.phase === 'recovery-required' && (
            <SmallButton onClick={() => request('reconcile')} label="Reconcile saved ownership" />
          )}
          <details style={{ maxHeight: 320, overflow: 'auto' }}>
            <summary style={{ cursor: 'pointer' }}>Inspect handoff and delivery evidence</summary>
            <p>
              Predecessor: <code>{operation.sourceSessionId}</code>
              <br />
              Successor: <code>{operation.successorSessionId}</code>
              <br />
              Artifact: <code>{operation.sealedArtifactPath ?? operation.artifactPath}</code>
            </p>
            <SmallButton
              label="Open predecessor transcript"
              onClick={() =>
                requestSessionWatch({
                  sessionId: operation.sourceSessionId,
                  title: 'Previous Fleet Manager',
                  provider,
                  cwd,
                })
              }
            />
            <p>
              The predecessor transcript remains available in session history. No uncertain delivery
              is replayed automatically.
            </p>
            {operation.deliveries.map((d) => (
              <details key={d.id}>
                <summary style={{ cursor: 'pointer' }}>
                  {d.kind}: {d.status}
                </summary>
                <pre
                  style={{
                    whiteSpace: 'pre-wrap',
                    overflowWrap: 'anywhere',
                    maxHeight: 240,
                    overflow: 'auto',
                  }}
                >
                  {d.text}
                </pre>
                {d.error && <p>{d.error}</p>}
                {(d.status === 'uncertain' ||
                  (d.status === 'pending' && operation.phase === 'recovery-required')) && (
                  <>
                    <p>
                      Inspect the destination transcript first. Sending again may duplicate agent
                      work.
                    </p>
                    <SmallButton
                      label={
                        d.id.startsWith('finish:')
                          ? 'I accounted for this result'
                          : 'I verified it arrived'
                      }
                      onClick={() =>
                        void managerReplacementRequest({
                          action: 'resolve-delivery',
                          operationId: operation.operationId,
                          deliveryId: d.id,
                          resolution: 'accepted',
                          acknowledgeDuplicateRisk: true,
                        })
                      }
                    />{' '}
                    <SmallButton
                      label="Send again — may duplicate work"
                      onClick={() =>
                        void managerReplacementRequest({
                          action: 'resolve-delivery',
                          operationId: operation.operationId,
                          deliveryId: d.id,
                          resolution: 'retry',
                          acknowledgeDuplicateRisk: true,
                        })
                      }
                    />
                  </>
                )}
              </details>
            ))}
          </details>
        </>
      )}
    </div>
  );
}
