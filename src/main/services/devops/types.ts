/**
 * Provider-agnostic DevOps types for git management + CI/CD.
 * All providers (Azure DevOps, GitHub, GitLab) normalize to these.
 */

// ── Pull Requests ──

export interface DevOpsPullRequest {
  id: string;
  accountId: string;
  provider: string;
  number: number;
  title: string;
  description: string;
  status: 'open' | 'draft' | 'closed' | 'merged' | 'abandoned';
  sourceBranch: string;
  targetBranch: string;
  author: string;
  reviewers: Reviewer[];
  url: string;
  created: string;
  updated: string;
  isDraft: boolean;
  mergeConflicts: boolean;
}

export interface Reviewer {
  name: string;
  vote: 'approved' | 'rejected' | 'waiting' | 'no_vote';
}

// ── Pipelines / Builds ──

export interface DevOpsPipeline {
  id: string;
  accountId: string;
  provider: string;
  name: string;
  status: 'running' | 'succeeded' | 'failed' | 'cancelled' | 'queued';
  sourceBranch: string;
  commitSha: string;
  author: string;
  url: string;
  startedAt: string;
  finishedAt: string;
  duration?: number; // seconds
}

// ── Repositories ──

export interface DevOpsRepo {
  id: string;
  accountId: string;
  provider: string;
  name: string;
  defaultBranch: string;
  url: string;
}

// ── Account ──

export interface DevOpsAccount {
  id: string;
  provider: string;
  label: string;
  config: Record<string, string>;
}

// ── Config field descriptors ──

export interface DevOpsConfigField {
  key: string;
  label: string;
  placeholder: string;
  type: 'text' | 'url' | 'email';
  required: boolean;
}

export interface DevOpsTokenField {
  label: string;
  placeholder: string;
  helpText?: string;
}

// ── Provider interface ──

export interface DevOpsProvider {
  readonly id: string;
  readonly name: string;
  readonly configFields: DevOpsConfigField[];
  readonly tokenField: DevOpsTokenField;

  validateCredentials(account: DevOpsAccount, token: string): Promise<boolean>;

  listRepos(account: DevOpsAccount, token: string): Promise<DevOpsRepo[]>;

  listPullRequests(account: DevOpsAccount, token: string, options?: {
    status?: 'open' | 'all';
    repoId?: string;
  }): Promise<DevOpsPullRequest[]>;

  listPipelines(account: DevOpsAccount, token: string, options?: {
    maxResults?: number;
  }): Promise<DevOpsPipeline[]>;
}
