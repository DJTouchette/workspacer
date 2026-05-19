/**
 * TrackerService — thin adapter over devdaemon's normalized issue API.
 *
 * Workspacer no longer stores Jira/ADO credentials or makes direct upstream
 * calls; the devdaemon owns all of that. This service exists so the existing
 * `tracker:*` IPC channels (and the renderer code that uses them) keep working
 * — every method delegates to `devdaemonClient`.
 *
 * Accounts here are virtual: one per devdaemon source that has auth configured.
 *   accountId  = "dd:jira" | "dd:ado"
 *   provider   = "jira" | "ado"
 *
 * NOTE: ADO work items are not exposed by the daemon today (only PRs + pipeline
 * runs are). The "ado" virtual account therefore returns empty issue lists.
 */
import * as devdaemon from '../devdaemon/client';
import type {
  DevdaemonIssue,
  DevdaemonProject,
  DevdaemonSource,
  DevdaemonTransition,
} from '../devdaemon/types';
import type {
  TrackerAccount,
  TrackerProject,
  TrackerIssue,
  TrackerStatus,
  TrackerTransition,
  ListIssuesOptions,
  ConfigField,
  TokenField,
} from './types';

const PROVIDERS: Array<{ id: DevdaemonSource; name: string }> = [
  { id: 'jira', name: 'Jira' },
  { id: 'ado', name: 'Azure DevOps' },
];

const ACCOUNT_ID_PREFIX = 'dd:';

function sourceFromAccountId(accountId: string): DevdaemonSource {
  const src = accountId.startsWith(ACCOUNT_ID_PREFIX)
    ? accountId.slice(ACCOUNT_ID_PREFIX.length)
    : accountId;
  if (src !== 'jira' && src !== 'ado') {
    throw new Error(`unknown devdaemon source: ${src}`);
  }
  return src;
}

function projectKeyFromIssueKey(key: string): string {
  const idx = key.indexOf('-');
  return idx > 0 ? key.slice(0, idx) : '';
}

function mapStatusCategory(c: string): TrackerIssue['statusCategory'] {
  if (c === 'done' || c === 'in_progress') return c;
  return 'todo';
}

function mapIssue(raw: DevdaemonIssue): TrackerIssue {
  return {
    id: raw.id,
    key: raw.key,
    title: raw.title,
    description: raw.description ?? '',
    status: raw.status.name,
    statusCategory: mapStatusCategory(raw.status.category),
    assignee: raw.assignee?.display_name,
    priority: raw.priority,
    type: 'Task',
    labels: raw.labels ?? [],
    provider: raw.source,
    accountId: ACCOUNT_ID_PREFIX + raw.source,
    projectKey: projectKeyFromIssueKey(raw.key),
    url: raw.url,
    created: raw.created_at,
    updated: raw.updated_at,
  };
}

function mapProject(raw: DevdaemonProject): TrackerProject {
  return {
    id: raw.id,
    key: raw.key,
    name: raw.name,
    provider: raw.source,
    accountId: ACCOUNT_ID_PREFIX + raw.source,
  };
}

function mapTransition(raw: DevdaemonTransition): TrackerTransition {
  return {
    id: raw.id,
    name: raw.name,
    to: {
      id: raw.id,
      name: raw.to.name,
      category: mapStatusCategory(raw.to.category),
    },
  };
}

class TrackerService {
  async getProviderList(): Promise<
    Array<{ id: string; name: string; configFields: ConfigField[]; tokenField: TokenField }>
  > {
    return PROVIDERS.map((p) => ({
      id: p.id,
      name: p.name,
      configFields: [],
      tokenField: {
        label: 'Credentials managed by devdaemon',
        placeholder: '',
        helpText: `Run \`devdaemon auth ${p.id} --${p.id === 'jira' ? 'token' : 'pat'} <token>\``,
      },
    }));
  }

  /** Return one virtual account per source the daemon has creds for. */
  async getAccounts(): Promise<TrackerAccount[]> {
    let status: { jira: string; ado: string };
    try {
      const got = await devdaemon.getAuthStatus();
      status = { jira: got.jira, ado: got.ado };
    } catch {
      return [];
    }
    const accounts: TrackerAccount[] = [];
    if (status.jira === 'ok') {
      accounts.push({
        id: ACCOUNT_ID_PREFIX + 'jira',
        provider: 'jira',
        label: 'Jira (devdaemon)',
        config: {},
        pinnedProjects: [],
      });
    }
    if (status.ado === 'ok') {
      accounts.push({
        id: ACCOUNT_ID_PREFIX + 'ado',
        provider: 'ado',
        label: 'Azure DevOps (devdaemon)',
        config: {},
        pinnedProjects: [],
      });
    }
    return accounts;
  }

  // Credentials are managed by the devdaemon CLI / dashboard, not Workspacer.
  async addAccount(
    _provider: string,
    _label: string,
    _config: Record<string, string>,
    _token: string,
  ): Promise<TrackerAccount> {
    throw new Error(
      'Tracker accounts are managed by devdaemon. Run `devdaemon auth jira --token <token>` or open the Daemon dashboard.',
    );
  }
  async updateAccount(_accountId: string, _updates: unknown): Promise<TrackerAccount> {
    throw new Error('Tracker accounts are managed by devdaemon.');
  }
  removeAccount(_accountId: string): void {
    throw new Error('Tracker accounts are managed by devdaemon.');
  }

  async listProjects(accountId: string): Promise<TrackerProject[]> {
    const source = sourceFromAccountId(accountId);
    const projects = await devdaemon.listProjects(source);
    return projects.map(mapProject);
  }

  async listIssues(accountId: string, options: ListIssuesOptions): Promise<TrackerIssue[]> {
    const source = sourceFromAccountId(accountId);
    if (source === 'ado') return []; // daemon doesn't expose ADO work items
    const issues = await devdaemon.searchIssues({
      sources: [source],
      query: options.query,
      assignee: options.assignedToMe ? 'me' : undefined,
      status: options.status,
    });
    if (options.projectKey) {
      return issues.filter((i) => projectKeyFromIssueKey(i.key) === options.projectKey).map(mapIssue);
    }
    return issues.map(mapIssue);
  }

  async getIssue(accountId: string, issueKey: string): Promise<TrackerIssue | null> {
    const source = sourceFromAccountId(accountId);
    if (source === 'ado') return null;
    try {
      const issue = await devdaemon.getIssue(`${source}:${issueKey}`);
      return mapIssue(issue);
    } catch {
      return null;
    }
  }

  async searchIssues(accountId: string, query: string): Promise<TrackerIssue[]> {
    const source = sourceFromAccountId(accountId);
    if (source === 'ado') return [];
    const issues = await devdaemon.searchIssues({ sources: [source], query });
    return issues.map(mapIssue);
  }

  async listStatuses(_accountId: string, _projectKey: string): Promise<TrackerStatus[]> {
    // Daemon doesn't expose a /statuses endpoint; renderer filter UI relies on
    // status names returned with issues, so an empty list is fine here.
    return [];
  }

  async getTransitions(accountId: string, issueKey: string): Promise<TrackerTransition[]> {
    if (sourceFromAccountId(accountId) !== 'jira') return [];
    const transitions = await devdaemon.getTransitions(`jira:${issueKey}`);
    return transitions.map(mapTransition);
  }

  async transitionIssue(accountId: string, issueKey: string, transitionId: string): Promise<void> {
    if (sourceFromAccountId(accountId) !== 'jira') {
      throw new Error('Transitions are only supported on Jira issues.');
    }
    await devdaemon.applyTransition(`jira:${issueKey}`, transitionId);
  }

  /** Resolve an issue key across configured sources — daemon auto-routes by shape. */
  async resolveIssueKey(issueKey: string): Promise<TrackerIssue | null> {
    try {
      const issue = await devdaemon.getIssue(issueKey);
      return mapIssue(issue);
    } catch {
      return null;
    }
  }

  async validateCredentials(accountId: string): Promise<boolean> {
    const source = sourceFromAccountId(accountId);
    try {
      const status = await devdaemon.getAuthStatus();
      return status[source] === 'ok';
    } catch {
      return false;
    }
  }
}

export const trackerService = new TrackerService();
