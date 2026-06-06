export type SessionAmbientState = 'idle' | 'thinking' | 'streaming' | 'waiting_input' | 'waiting_approval';

export interface ToolCall {
  id: string;
  name: string;
  input: any;
  response?: any;
  status: 'running' | 'complete' | 'failed';
  startedAt: number;
  completedAt?: number;
}

export interface ConversationTurn {
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  toolCalls?: ToolCall[];
}

export interface FileChange {
  path: string;
  toolName: string;
  input: any;
  timestamp: number;
}

export interface PendingApproval {
  toolName: string;
  toolInput: any;
  suggestions?: string[];
  timestamp: number;
}

export interface PendingQuestionOption {
  label: string;
  description?: string;
}

export interface PendingQuestion {
  question: string;
  header?: string;
  multi_select?: boolean;
  options: PendingQuestionOption[];
}

export interface SubagentInfo {
  id: string;
  type: string;
  status: 'running' | 'complete';
  startedAt: number;
  completedAt?: number;
  // Live enrichment from the subagent's transcript (main-process workflowWatcher)
  description?: string;
  model?: string;
  tokens?: number;
  toolCalls?: number;
  lastToolName?: string;
  lastToolSummary?: string;
}

// ── Workflow runs (mirrors src/main/services/workflowWatcher.ts) ──

export interface WorkflowPhaseInfo {
  title: string;
  detail?: string;
}

export interface WorkflowAgentInfo {
  id: string;
  label?: string;
  phaseTitle?: string;
  model?: string;
  status: 'queued' | 'running' | 'done' | 'failed';
  startedAt?: number;
  completedAt?: number;
  durationMs?: number;
  tokens: number;
  toolCalls: number;
  lastToolName?: string;
  lastToolSummary?: string;
  promptPreview?: string;
  resultPreview?: string;
}

export interface WorkflowRunInfo {
  runId: string;
  name?: string;
  description?: string;
  status: 'running' | 'completed' | 'failed';
  startedAt: number;
  completedAt?: number;
  durationMs?: number;
  phases: WorkflowPhaseInfo[];
  agents: WorkflowAgentInfo[];
  totalTokens?: number;
  totalToolCalls?: number;
}

/** Token / cost / context usage, parsed from the transcript in the main process. */
export interface SessionUsage {
  model: string | null;
  contextTokens: number; // latest turn's input side (point-in-time)
  contextLimit: number; // tokens the model's window holds
  totalInputTokens: number; // cumulative (incl. cache)
  totalOutputTokens: number; // cumulative
  costUSD: number; // cumulative
}

export interface ClaudeSessionSnapshot {
  sessionId: string;
  cwd: string;
  ptyId: string;

  status: 'starting' | 'active' | 'ended';
  conversation: ConversationTurn[];
  activeToolCalls: ToolCall[];
  completedToolCalls: ToolCall[];
  fileChanges: FileChange[];
  pendingApproval: PendingApproval | null;
  pendingQuestions: PendingQuestion[] | null;
  subagents: SubagentInfo[];
  workflows: WorkflowRunInfo[];

  ambientState: SessionAmbientState;
  lastActivity: number;
  totalToolCalls: number;
  usage: SessionUsage | null;

  /** Human label set at spawn time (e.g. by a supervisor naming a worker). */
  label?: string;
  /** Session id of the agent that spawned this one — drives nesting in the UI. */
  parentSessionId?: string;
}
