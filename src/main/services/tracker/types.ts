/**
 * Provider-agnostic issue tracker types.
 * All providers (Jira, Linear, Trello, etc.) normalize to these.
 */

// ── Normalized domain types ──

export interface TrackerProject {
  id: string;
  key: string;        // e.g. "PROJ"
  name: string;
  avatarUrl?: string;
  provider: string;   // provider id that owns this
  accountId: string;  // account id that owns this
}

export interface TrackerIssue {
  id: string;
  key: string;           // e.g. "PROJ-123"
  title: string;
  description: string;   // plain text or markdown
  status: string;        // display name
  statusCategory: 'todo' | 'in_progress' | 'done';
  assignee?: string;
  priority?: string;
  type: string;          // bug, story, task, epic, etc.
  labels: string[];
  parentKey?: string;
  provider: string;
  accountId: string;
  projectKey: string;
  url: string;           // web URL to the issue
  created: string;       // ISO 8601
  updated: string;       // ISO 8601
}

export interface TrackerStatus {
  id: string;
  name: string;
  category: 'todo' | 'in_progress' | 'done';
}

export interface TrackerTransition {
  id: string;
  name: string;
  to: TrackerStatus;
}

// ── Account configuration ──

export interface TrackerAccount {
  id: string;
  provider: string;      // 'jira' | 'linear' | 'trello'
  label: string;         // user-friendly name, e.g. "Work Jira"
  /** Provider-specific config (host URL, email, etc.) — no secrets here */
  config: Record<string, string>;
  /** Projects the user has selected to track (empty = all) */
  pinnedProjects: string[];
}

// ── List options ──

export interface ListIssuesOptions {
  projectKey?: string;
  status?: string;
  assignedToMe?: boolean;
  query?: string;
  maxResults?: number;
  startAt?: number;
}

// ── Provider interface (facade) ──

export interface TrackerProvider {
  /** Unique provider identifier, e.g. 'jira' */
  readonly id: string;
  /** Human-readable name, e.g. 'Jira' */
  readonly name: string;
  /** Fields needed to configure an account (shown in onboarding UI) */
  readonly configFields: ConfigField[];

  /** Validate credentials — returns true if auth works */
  validateCredentials(account: TrackerAccount, token: string): Promise<boolean>;

  /** List all projects accessible by this account */
  listProjects(account: TrackerAccount, token: string): Promise<TrackerProject[]>;

  /** List issues with filtering */
  listIssues(account: TrackerAccount, token: string, options: ListIssuesOptions): Promise<TrackerIssue[]>;

  /** Get a single issue by key (e.g. "PROJ-123") */
  getIssue(account: TrackerAccount, token: string, issueKey: string): Promise<TrackerIssue | null>;

  /** Free-text search across issues */
  searchIssues(account: TrackerAccount, token: string, query: string): Promise<TrackerIssue[]>;

  /** List available statuses for a project */
  listStatuses(account: TrackerAccount, token: string, projectKey: string): Promise<TrackerStatus[]>;

  /** Get available transitions for a specific issue */
  getTransitions(account: TrackerAccount, token: string, issueKey: string): Promise<TrackerTransition[]>;

  /** Transition an issue to a new status */
  transitionIssue(account: TrackerAccount, token: string, issueKey: string, transitionId: string): Promise<void>;
}

// ── Config field descriptor (drives the onboarding form) ──

export interface ConfigField {
  key: string;          // maps to TrackerAccount.config[key]
  label: string;        // form label
  placeholder: string;
  type: 'text' | 'url' | 'email';
  required: boolean;
}

/** The token field is always separate (stored in safeStorage, not in config) */
export interface TokenField {
  label: string;
  placeholder: string;
  helpText?: string;
}
