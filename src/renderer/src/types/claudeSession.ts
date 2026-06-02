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

  ambientState: SessionAmbientState;
  lastActivity: number;
  totalToolCalls: number;
  usage: SessionUsage | null;
}
