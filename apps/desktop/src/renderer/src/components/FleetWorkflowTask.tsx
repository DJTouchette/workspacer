import React from 'react';
import type { DispatchTask } from '../../../main/shared/dispatchHistory';
import { reviewPolicy } from '../../../main/shared/fleetWorkflow';
export default function FleetWorkflowTask({ task }: { task: DispatchTask }) {
  const pin = task.workflow;
  if (!pin)
    return <p className="recent-note">Fleet workflow unknown — historical or standalone task.</p>;
  return (
    <section aria-label="Pinned Fleet workflow" style={{ overflowWrap: 'anywhere' }}>
      <p>
        <strong>{pin.definition.name}</strong> · revision {pin.definition.revision}
        <br />
        {reviewPolicy(pin.definition)}
      </p>
      <details>
        <summary>Frozen policy and steps</summary>
        <p>
          Snapshot {pin.hash}. Edits affect new tasks only. Completed means a valid result contract
          was received; reported outcomes remain separate.
        </p>
        <ol>
          {pin.steps.map((run, i) => (
            <li key={run.id} style={{ marginBottom: 8 }}>
              <strong>{pin.definition.steps[i].label}</strong> · {run.state} ·{' '}
              {pin.definition.steps[i].stage} / {pin.definition.steps[i].role}
              {run.reason && <p>{run.reason}</p>}
              {run.sessionId && <p>Worker {run.sessionId}</p>}
              {run.outcome !== undefined && (
                <details>
                  <summary>Reported outcome (not an inferred pass)</summary>
                  <pre style={{ whiteSpace: 'pre-wrap' }}>
                    {JSON.stringify(run.outcome, null, 2)}
                  </pre>
                </details>
              )}
              <details>
                <summary>Dispatch contract</summary>
                <p>{pin.definition.steps[i].instructions}</p>
                <pre style={{ whiteSpace: 'pre-wrap' }}>
                  {pin.templates[pin.definition.steps[i].template]?.body}
                </pre>
                <pre style={{ whiteSpace: 'pre-wrap' }}>
                  {JSON.stringify(
                    pin.templates[pin.definition.steps[i].template]?.resultSchema,
                    null,
                    2,
                  )}
                </pre>
              </details>
            </li>
          ))}
        </ol>
      </details>
    </section>
  );
}
