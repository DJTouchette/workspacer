import { boundIntentReport } from './intentReport';
import type { IntentCompletionView } from './intentCompletion';
import type { IntentAutomationRequest, IntentAutomationResponse } from './intentAutomation';
import type { IntentEvidenceRequest, IntentEvidenceResponse } from './intentEvidence';
import type { IntentSourceRequest, IntentSourceResponse } from './intentSources';
import type { IntentProject, IntentProjectRequest, IntentProjectResponse } from './intentProject';
import type { IntentKnowledgeRequest, IntentKnowledgeResponse } from './intentKnowledge';
import type { IntentArtifactRequest, IntentArtifactResponse } from './intentArtifacts';
import type {
  IntentControlRequest,
  IntentControlResponse,
  IntentReconciliation,
  IntentReconciliationInput,
} from './intentControl';

/** Durable feature state. Separate from AgentWorkspace, which owns pane layout. */
export const INTENT_STATUSES = ['draft', 'active', 'review', 'complete'] as const;
export type IntentStatus = (typeof INTENT_STATUSES)[number];

export interface IntentFields {
  title: string;
  outcome: string;
  constraints: string;
  successCriteria: string;
  sourceUrl: string;
  status: IntentStatus;
}

export interface IntentWorkspace extends IntentFields {
  id: string;
  projectRoot: string;
  projectId?: string;
  repositoryId?: string;
  revision: number;
  createdAt: string;
  updatedAt: string;
}

export interface IntentRevision {
  revision: number;
  at: string;
  reason: string;
  snapshot: IntentWorkspace;
}

export interface IntentSessionRef {
  sessionId: string;
  /** Empty means the selected host; a peer ID is part of session identity. */
  hub: string;
  label: string;
  provider: string;
  cwd: string;
}

export interface IntentObservation {
  completionIdle?: boolean;
  state: string;
  summary: string;
  cwd: string;
  observedAt: string;
}

export interface IntentExecution {
  id: string;
  workspaceId: string;
  intentRevision: number;
  kind: 'launch' | 'attached';
  completionContract?: 1;
  state: 'launching' | 'linked' | 'unknown';
  task: string;
  /** Immutable launch payload. Null for tracking-only associations. */
  contextPacket: string | null;
  session: IntentSessionRef | null;
  lastObservation: IntentObservation | null;
  createdAt: string;
  updatedAt: string;
}

export interface IntentWorkLink {
  id: string;
  workspaceId: string;
  kind: 'branch' | 'pull-request';
  target: string;
  createdAt: string;
}

export interface IntentDirectionAttempt {
  id: string;
  /** Unknown is persisted before I/O, including while delivery is in flight. */
  status: 'accepted' | 'failed' | 'unknown';
  startedAt: string;
  finishedAt?: string;
  detail: string;
}

export interface IntentDirection {
  id: string;
  workspaceId: string;
  executionId: string;
  intentRevision: number;
  target: IntentSessionRef;
  author: 'user';
  text: string;
  /** Exact immutable message reviewed before sending. */
  packet: string;
  createdAt: string;
  supersedesId?: string;
  supersededBy?: string;
  attempts: IntentDirectionAttempt[];
  reconciliations?: IntentReconciliation[];
}

export function buildIntentDirectionContext(
  workspace: IntentWorkspace,
  direction: Pick<IntentDirection, 'id' | 'executionId' | 'text' | 'supersedesId'>,
): string {
  return [
    `Workspacer direction: ${workspace.title}`,
    `Workspace ID: ${workspace.id}`,
    `Intent revision: ${workspace.revision}`,
    `Execution ID: ${direction.executionId}`,
    `Direction ID: ${direction.id}`,
    direction.supersedesId
      ? `Replaces direction: ${direction.supersedesId}. Earlier actions are not undone by this message.`
      : '',
    '',
    'Saved intent for this direction (user-authored):',
    JSON.stringify(
      {
        outcome: workspace.outcome,
        constraints: workspace.constraints,
        successCriteria: workspace.successCriteria,
      },
      null,
      2,
    ),
    '',
    'Direction from the user:',
    direction.text,
    '',
    'Explain any conflict with work already performed and report what you changed or still need clarified. Delivery of this message does not establish that this direction has been applied.',
  ]
    .filter((line) => line !== undefined)
    .join('\n');
}

/** Minimum host-owned snapshot needed for progress; never accepted in request payloads. */
export interface IntentLiveSession {
  sessionId: string;
  hub?: string;
  hubOffline?: boolean;
  parentSessionId?: string;
  status?: string;
  ambientState?: string;
  cwd?: string;
  liveCwd?: string;
  label?: string;
  provider?: string;
  subagents?: readonly { status: string }[];
  activeToolCalls?: readonly unknown[];
  conversation?: readonly { role: string; content: string }[];
  pendingQuestions?: readonly { question: string }[] | null;
  pendingApproval?: { toolName: string } | null;
}

export function intentObservation(session: IntentLiveSession, now: string): IntentObservation {
  const state =
    session.status === 'ended' || session.status === 'stopped'
      ? 'stopped'
      : session.pendingApproval || session.pendingQuestions?.length
        ? 'blocked'
        : session.ambientState || 'unknown';
  let assistantText = '';
  const conversation = session.conversation ?? [];
  for (let i = conversation.length - 1; i >= 0; i--) {
    if (conversation[i].role === 'assistant' && conversation[i].content.trim()) {
      assistantText = conversation[i].content;
      break;
    }
  }
  const summary =
    session.pendingQuestions?.[0]?.question ||
    (session.pendingApproval ? `Approval needed: ${session.pendingApproval.toolName}` : '') ||
    assistantText ||
    '';
  return {
    state,
    summary: boundIntentReport(summary).report,
    cwd: session.liveCwd || session.cwd || '',
    observedAt: now,
  };
}

/** Compiled once from a saved revision; the first-message payload is inspectable later. */
export function buildIntentContext(
  workspace: IntentWorkspace,
  executionId: string,
  task: string,
): string {
  return [
    `Workspacer intent: ${workspace.title}`,
    `Workspace ID: ${workspace.id}`,
    `Intent revision: ${workspace.revision}`,
    `Execution ID: ${executionId}`,
    `Project directory: ${workspace.projectRoot}`,
    '',
    'Read applicable repository instructions (including AGENTS.md) and existing project documentation before changing code. Consult Rivet project context if it is available. Surface conflicts between the requested work and project conventions.',
    '',
    'Saved intent at launch (user-authored):',
    JSON.stringify(
      {
        outcome: workspace.outcome,
        constraints: workspace.constraints,
        successCriteria: workspace.successCriteria,
        sourceUrl: workspace.sourceUrl,
      },
      null,
      2,
    ),
    `Finish with an outcome summary containing actual checks, changed artifacts, caveats and unresolved questions. Keep the entire final reply below 3500 characters. Never include credentials or tool transcripts. When ready for human review, with no outstanding workers, end with a fenced intent-report JSON containing executionId: ${executionId}, revision: ${workspace.revision}, state: "review", summary (string), checks, artifacts, caveats, followUps (arrays of strings). If dedicated run instructions specify runId, use that contract instead. Agent reports do not verify criteria or approve outcomes.`,
    'The source URL is a reference; its contents were not imported into this packet.',
    '',
    'Requested work:',
    task,
    '',
    'Report the result against the success criteria, the checks actually run, and any unresolved questions or blockers. This recorded launch snapshot never changes. Later directions arrive as separate messages.',
  ].join('\n');
}

export type IntentWorkspaceRequest =
  | { action: 'completionProposals'; id: string }
  | IntentAutomationRequest
  | IntentEvidenceRequest
  | IntentSourceRequest
  | IntentProjectRequest
  | IntentKnowledgeRequest
  | IntentArtifactRequest
  | IntentControlRequest
  | ({ action: 'reconcileDirection'; id: string; directionId: string } & IntentReconciliationInput)
  | { action: 'list' }
  | { action: 'create'; projectRoot: string; fields: IntentFields }
  | {
      action: 'update';
      id: string;
      expectedRevision: number;
      expectedUpdatedAt?: string;
      fields: IntentFields;
      reason: string;
    }
  | { action: 'history'; id: string }
  | { action: 'directions'; id: string }
  | {
      action: 'prepareDirection';
      id: string;
      directionId: string;
      executionId: string;
      expectedRevision: number;
      text: string;
      supersedesId?: string;
    }
  | { action: 'sendDirection'; id: string; directionId: string; attemptId: string }
  | { action: 'executions'; id: string }
  | {
      action: 'prepareExecution';
      id: string;
      expectedRevision: number;
      executionId: string;
      task: string;
    }
  | { action: 'linkExecution'; id: string; executionId: string; session: IntentSessionRef }
  | { action: 'markExecutionUnknown'; id: string; executionId: string }
  | { action: 'attachSession'; id: string; expectedRevision: number; session: IntentSessionRef }
  | { action: 'addWorkLink'; id: string; kind: IntentWorkLink['kind']; target: string };

export type IntentWorkspaceResponse =
  | IntentCompletionView
  | IntentAutomationResponse
  | IntentEvidenceResponse
  | IntentSourceResponse
  | IntentProjectResponse
  | IntentKnowledgeResponse
  | IntentArtifactResponse
  | IntentControlResponse
  | { action: 'reconcileDirection'; direction: IntentDirection }
  | {
      action: 'list';
      workspaces: IntentWorkspace[];
      executionIndex?: Record<string, IntentSessionRef[]>;
      projects?: IntentProject[];
    }
  | { action: 'create' | 'update'; workspace: IntentWorkspace }
  | {
      action: 'history';
      revisions: IntentRevision[];
      statusEvents?: { at: string; status: IntentStatus; reason: string }[];
    }
  | { action: 'directions'; directions: IntentDirection[] }
  | { action: 'prepareDirection'; direction: IntentDirection; created: boolean }
  | { action: 'sendDirection'; direction: IntentDirection; dispatched: boolean }
  | {
      action: 'executions';
      executions: IntentExecution[];
      links: IntentWorkLink[];
      captureWarning?: string;
    }
  | { action: 'prepareExecution'; execution: IntentExecution; created: boolean }
  | {
      action: 'linkExecution' | 'markExecutionUnknown' | 'attachSession';
      execution: IntentExecution;
    }
  | { action: 'addWorkLink'; link: IntentWorkLink };
