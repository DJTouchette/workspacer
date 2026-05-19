/**
 * TypeScript mirrors of devdaemon's normalized JSON shapes.
 * Source of truth: C:\Users\DamienTouchette\work\biai\internal\core\types.go
 */

export type DevdaemonSource = 'jira' | 'ado';
export type StatusCategory = 'todo' | 'in_progress' | 'done' | 'blocked' | 'other';

export interface DevdaemonUser {
  id: string;
  email: string;
  display_name: string;
}

export interface DevdaemonStatus {
  name: string;
  category: StatusCategory;
}

export interface DevdaemonIssue {
  id: string;            // "jira:PROJ-123" | "ado:1234"
  source: DevdaemonSource;
  key: string;
  title: string;
  description?: string;
  status: DevdaemonStatus;
  priority?: string;
  assignee?: DevdaemonUser | null;
  reporter?: DevdaemonUser | null;
  url: string;
  created_at: string;    // ISO-8601
  updated_at: string;
  labels?: string[];
  raw?: unknown;
}

export interface DevdaemonRepository {
  id: string;
  name: string;
  project: string;
  url: string;
  default_branch?: string;
}

export interface DevdaemonProject {
  id: string;        // "jira:CASS" | "ado:Leroy"
  source: DevdaemonSource;
  key: string;
  name: string;
  url?: string;
}

export interface DevdaemonTransition {
  id: string;
  name: string;
  to: DevdaemonStatus;
}

export interface DevdaemonPullRequest {
  id: string;            // "ado:12345"
  source: 'ado';
  number: number;
  title: string;
  status: 'active' | 'completed' | 'abandoned';
  author?: DevdaemonUser | null;
  source_branch: string;
  target_branch: string;
  repository: string;
  url: string;
  created_at: string;
  updated_at: string;
  is_draft: boolean;
  reviewers?: DevdaemonUser[];
}

export interface DevdaemonPipelineRun {
  id: string;            // "ado:run:6789"
  source: 'ado';
  pipeline_name: string;
  pipeline_id: number;
  run_number: number;
  status: string;        // "inProgress" | "completed" | "cancelling" | ...
  result: string;        // "succeeded" | "failed" | "canceled" | ""
  url: string;
  branch: string;
  commit_id: string;
  started_at: string;
  finished_at?: string | null;
}

export interface DevdaemonHealth {
  status: 'ok';
  jira: 'ok' | 'missing';
  ado: 'ok' | 'missing';
  claudemon: 'connected' | 'absent';
  subscribers: number;
}

export interface DevdaemonAuthStatus {
  jira: 'ok' | 'missing';
  ado: 'ok' | 'missing';
  claudemon: 'connected' | 'absent';
}

// Event bus payload — fields match internal/bus/event.go.
export interface DevdaemonEvent {
  id: string;
  type: string;       // e.g. "jira.issue.updated", "ado.pr.created", "plugin:xyz.foo"
  source: string;     // "jira" | "ado" | "plugin:<name>"
  time: string;
  subject: string;
  data: unknown;
}
