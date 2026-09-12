/**
 * Bridges workspacer's live workflow telemetry onto the hub event bus, so the
 * rules engine (and any plugin) can react to it. The `workflowWatcher` produces
 * rich per-run / per-agent state, but it only ever reached the renderer — this
 * module republishes the *transitions* onto the bus:
 *
 *   workflow.started          a run is first seen running
 *   workflow.completed        a run finishes ok
 *   workflow.failed           a run fails
 *   workflow.agent.finished   one agent in a run reaches done|failed
 *
 * We deliberately publish only on state transitions, never on every 1s watcher
 * tick, so the bus stays quiet. Agent attention (needs-approval / question /
 * input / Stop) is already published by the Go claudemon bridge as
 * `agent.state_changed`, and cost/budget is covered by the rules engine's
 * `agents.list` poll loop — so neither is duplicated here.
 */
import { publishToHub } from './hubClient';
import { isRemoteShareEnabled } from './hubDaemon';
import { compactClaudeSnapshotForBackground } from '../shared/compactClaudeSnapshot';
import { createWorkflowTelemetry } from './workflowTelemetryCore';
import type { ClaudeSessionSnapshot } from './claudeSessionStore';

/**
 * Publish a full session snapshot onto the bus as `agent.snapshot`, so the web
 * build's renderer (which has no Electron IPC) gets the same rich per-session
 * state — transcript, tool calls, fleet/workflow detail — that the desktop gets
 * over `claude-session:update`. Gated on remote sharing: when it's off there is
 * no web consumer, so we skip the extra serialization entirely and the
 * desktop-only path is unchanged.
 */
export function publishSnapshot(makeSnapshot: () => ClaudeSessionSnapshot): void {
  // Takes a factory, not a snapshot: the caller's `{ ...session }` copy is
  // itself part of the cost this gate exists to avoid, and building it before
  // the gate meant every flush of every session paid for it whether or not
  // anything was listening.
  if (!isRemoteShareEnabled()) return;
  // Bounded, not whole. This fires on every flush of every session (~60/s while
  // one streams), and it used to carry the entire transcript each time — so a
  // fleet of long-running sessions pushed megabytes per second at every bus
  // client. The window carries `conversationOffset`, the absolute index of its
  // first turn, which is what lets a client holding full history splice it in
  // (mergeConversationWindow) instead of the host resending everything.
  publishToHub({
    type: 'agent.snapshot',
    data: compactClaudeSnapshotForBackground(makeSnapshot()),
  });
}

const telemetry=createWorkflowTelemetry(publishToHub);
export const publishWorkflowRuns=telemetry.publishWorkflowRuns;
export const forgetSession=telemetry.forgetSession;
