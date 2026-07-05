import React, { useEffect, useMemo, useState } from 'react';
import { useAttention } from '../contexts/AttentionContext';
import { WORKFLOW_OPEN_EVENT } from '../lib/workflowBus';
import { WorkflowTimeline } from './claude/WorkflowTimeline';

/**
 * App-level host for the full-height workflow timeline. Listens for
 * `requestWorkflow(runId)` (from a WorkflowRunCard's expand button) and renders
 * the timeline for that run, re-reading it LIVE from the current snapshots so it
 * keeps updating while the workflow runs. Renders nothing when closed or when the
 * run isn't present (e.g. its session was cleared).
 */
export const WorkflowOverlay: React.FC = () => {
  const { snapshotBySession } = useAttention();
  const [runId, setRunId] = useState<string | null>(null);

  useEffect(() => {
    const onOpen = (e: Event) => setRunId((e as CustomEvent<string>).detail);
    window.addEventListener(WORKFLOW_OPEN_EVENT, onOpen);
    return () => window.removeEventListener(WORKFLOW_OPEN_EVENT, onOpen);
  }, []);

  const found = useMemo(() => {
    if (!runId) return undefined;
    for (const [sid, snap] of Object.entries(snapshotBySession)) {
      const r = snap?.workflows?.find((w) => w.runId === runId);
      if (r) return { sessionId: sid, run: r };
    }
    return undefined;
  }, [runId, snapshotBySession]);

  if (!runId || !found) return null;
  return (
    <WorkflowTimeline sessionId={found.sessionId} run={found.run} onClose={() => setRunId(null)} />
  );
};
