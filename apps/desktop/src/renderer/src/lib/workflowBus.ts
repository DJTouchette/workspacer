/**
 * Cross-component bus for opening a workflow run in the full-height Workflow
 * timeline overlay.
 *
 * A `WorkflowRunCard` (rendered deep in an agent's conversation, with no
 * sessionId in scope) calls `requestWorkflow(runId)`. A single `WorkflowOverlay`
 * host mounted near the app root listens, then re-reads the LIVE run from the
 * current snapshots by runId (run ids are globally unique, `wf_<uuid>`), so the
 * overlay keeps updating while the workflow runs — rather than freezing a
 * snapshot captured at click time.
 */

export const WORKFLOW_OPEN_EVENT = 'workflow:open';

/** Ask the app to open the full-height timeline for this workflow run. */
export function requestWorkflow(runId: string): void {
  window.dispatchEvent(new CustomEvent(WORKFLOW_OPEN_EVENT, { detail: runId }));
}
