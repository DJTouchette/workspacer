/**
 * Jira Cloud / Data Center provider.
 * Implements the TrackerProvider interface against the Jira REST API v2.
 */
import type {
  TrackerProvider,
  TrackerAccount,
  TrackerProject,
  TrackerIssue,
  TrackerStatus,
  ListIssuesOptions,
  ConfigField,
  TokenField,
} from './types';

// ── Helpers ──

function baseUrl(account: TrackerAccount): string {
  let url = account.config.url ?? '';
  if (url.endsWith('/')) url = url.slice(0, -1);
  return url;
}

function headers(account: TrackerAccount, token: string): Record<string, string> {
  const email = account.config.email ?? '';
  return {
    Authorization: `Basic ${Buffer.from(`${email}:${token}`).toString('base64')}`,
    Accept: 'application/json',
    'Content-Type': 'application/json',
  };
}

async function jiraFetch(account: TrackerAccount, token: string, path: string): Promise<any> {
  const url = `${baseUrl(account)}/rest/api/3${path}`;
  const res = await fetch(url, { headers: headers(account, token) });
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new Error(`Jira API ${res.status}: ${body.slice(0, 200)}`);
  }
  return res.json();
}

function mapStatusCategory(cat: any): 'todo' | 'in_progress' | 'done' {
  const key = cat?.key ?? cat?.name ?? '';
  if (key === 'done') return 'done';
  if (key === 'indeterminate' || key === 'in_progress') return 'in_progress';
  return 'todo';
}

/** Extract plain text from Atlassian Document Format (v3 API) */
function adfToText(node: any): string {
  if (!node) return '';
  if (typeof node === 'string') return node;
  if (node.type === 'text') return node.text ?? '';
  if (node.type === 'hardBreak') return '\n';
  if (Array.isArray(node.content)) {
    const inner = node.content.map(adfToText).join('');
    // Add newlines after block-level nodes
    if (['paragraph', 'heading', 'bulletList', 'orderedList', 'listItem', 'blockquote', 'codeBlock', 'rule'].includes(node.type)) {
      return inner + '\n';
    }
    return inner;
  }
  return '';
}

function mapIssue(account: TrackerAccount, raw: any): TrackerIssue {
  const fields = raw.fields ?? {};
  const desc = fields.description;
  const descText = typeof desc === 'string' ? desc : adfToText(desc).trim();
  return {
    id: raw.id,
    key: raw.key,
    title: fields.summary ?? '',
    description: descText,
    status: fields.status?.name ?? 'Unknown',
    statusCategory: mapStatusCategory(fields.status?.statusCategory),
    assignee: fields.assignee?.displayName,
    priority: fields.priority?.name,
    type: fields.issuetype?.name ?? 'Task',
    labels: fields.labels ?? [],
    provider: 'jira',
    accountId: account.id,
    projectKey: fields.project?.key ?? raw.key?.split('-')[0] ?? '',
    url: `${baseUrl(account)}/browse/${raw.key}`,
    created: fields.created ?? '',
    updated: fields.updated ?? '',
  };
}

// ── Provider ──

export const jiraConfigFields: ConfigField[] = [
  {
    key: 'url',
    label: 'Jira URL',
    placeholder: 'https://yourcompany.atlassian.net',
    type: 'url',
    required: true,
  },
  {
    key: 'email',
    label: 'Email',
    placeholder: 'you@company.com',
    type: 'email',
    required: true,
  },
];

export const jiraTokenField: TokenField = {
  label: 'API Token',
  placeholder: 'Paste your Jira API token',
  helpText: 'Generate at https://id.atlassian.com/manage-profile/security/api-tokens',
};

export class JiraProvider implements TrackerProvider {
  readonly id = 'jira';
  readonly name = 'Jira';
  readonly configFields = jiraConfigFields;
  readonly tokenField = jiraTokenField;

  async validateCredentials(account: TrackerAccount, token: string): Promise<boolean> {
    try {
      await jiraFetch(account, token, '/myself');
      return true;
    } catch {
      return false;
    }
  }

  async listProjects(account: TrackerAccount, token: string): Promise<TrackerProject[]> {
    const data = await jiraFetch(account, token, '/project?recent=50');
    return (data as any[]).map((p) => ({
      id: p.id,
      key: p.key,
      name: p.name,
      avatarUrl: p.avatarUrls?.['32x32'],
      provider: 'jira',
      accountId: account.id,
    }));
  }

  async listIssues(account: TrackerAccount, token: string, options: ListIssuesOptions): Promise<TrackerIssue[]> {
    const clauses: string[] = [];
    if (options.projectKey) clauses.push(`project = "${options.projectKey}"`);
    if (options.status) clauses.push(`status = "${options.status}"`);
    if (options.assignedToMe) clauses.push('assignee = currentUser()');
    if (options.query) {
      // Detect issue key pattern (e.g. PL-43, PROJ-123)
      const isIssueKey = /^[A-Z][A-Z0-9]+-\d+$/i.test(options.query.trim());
      if (isIssueKey) {
        clauses.push(`key = "${options.query.trim().toUpperCase()}"`);
      } else {
        clauses.push(`text ~ "${options.query.replace(/"/g, '\\"')}"`);
      }
    }

    const jql = (clauses.length > 0 ? clauses.join(' AND ') + ' ' : '') + 'ORDER BY updated DESC';
    const maxResults = options.maxResults ?? 50;
    const startAt = options.startAt ?? 0;

    const data = await jiraFetch(
      account,
      token,
      `/search/jql?jql=${encodeURIComponent(jql)}&maxResults=${maxResults}&startAt=${startAt}&fields=summary,status,assignee,priority,issuetype,project,labels,created,updated,description`,
    );

    return (data.issues ?? []).map((raw: any) => mapIssue(account, raw));
  }

  async getIssue(account: TrackerAccount, token: string, issueKey: string): Promise<TrackerIssue | null> {
    try {
      const raw = await jiraFetch(account, token, `/issue/${encodeURIComponent(issueKey)}?fields=summary,status,assignee,priority,issuetype,project,labels,created,updated,description`);
      return mapIssue(account, raw);
    } catch {
      return null;
    }
  }

  async searchIssues(account: TrackerAccount, token: string, query: string): Promise<TrackerIssue[]> {
    return this.listIssues(account, token, { query, maxResults: 20 });
  }

  async listStatuses(account: TrackerAccount, token: string, projectKey: string): Promise<TrackerStatus[]> {
    const data = await jiraFetch(account, token, `/project/${encodeURIComponent(projectKey)}/statuses`);
    const statuses: TrackerStatus[] = [];
    const seen = new Set<string>();
    for (const issueType of data as any[]) {
      for (const s of issueType.statuses ?? []) {
        if (seen.has(s.id)) continue;
        seen.add(s.id);
        statuses.push({
          id: s.id,
          name: s.name,
          category: mapStatusCategory(s.statusCategory),
        });
      }
    }
    return statuses;
  }
}
