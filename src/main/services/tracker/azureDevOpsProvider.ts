/**
 * Azure DevOps provider.
 * Implements the TrackerProvider interface against the Azure DevOps REST API v7.0.
 */
import type {
  TrackerProvider,
  TrackerAccount,
  TrackerProject,
  TrackerIssue,
  TrackerStatus,
  TrackerTransition,
  TrackerPullRequest,
  TrackerPipeline,
  ListIssuesOptions,
  ConfigField,
  TokenField,
} from './types';

// ── Helpers ──

function org(account: TrackerAccount): string {
  let o = account.config.org ?? '';
  // Strip URL prefix if user pasted the full URL
  o = o.replace(/^https?:\/\/dev\.azure\.com\/?/i, '').replace(/\/$/, '');
  return o;
}

function project(account: TrackerAccount): string {
  return account.config.project ?? '';
}

function authHeaders(token: string): Record<string, string> {
  return {
    Authorization: `Basic ${Buffer.from(`:${token}`).toString('base64')}`,
    Accept: 'application/json',
    'Content-Type': 'application/json',
  };
}

function baseUrl(account: TrackerAccount): string {
  return `https://dev.azure.com/${org(account)}/${project(account)}/_apis`;
}

function orgBaseUrl(account: TrackerAccount): string {
  return `https://dev.azure.com/${org(account)}/_apis`;
}

async function adoFetch(url: string, token: string, init?: RequestInit): Promise<any> {
  const sep = url.includes('?') ? '&' : '?';
  const fullUrl = `${url}${sep}api-version=7.0`;
  const res = await fetch(fullUrl, {
    ...init,
    headers: { ...authHeaders(token), ...init?.headers },
  });
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new Error(`Azure DevOps API ${res.status}: ${body.slice(0, 200)}`);
  }
  if (res.status === 204) return null;
  return res.json().catch(() => null);
}

async function adoGet(url: string, token: string): Promise<any> {
  return adoFetch(url, token);
}

async function adoPost(url: string, token: string, body: any, contentType?: string): Promise<any> {
  return adoFetch(url, token, {
    method: 'POST',
    headers: contentType ? { 'Content-Type': contentType } : undefined,
    body: JSON.stringify(body),
  });
}

async function adoPatch(url: string, token: string, body: any): Promise<any> {
  return adoFetch(url, token, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json-patch+json' },
    body: JSON.stringify(body),
  });
}

/** Strip HTML tags to get plain text (Azure DevOps descriptions are HTML) */
function stripHtml(html: string): string {
  if (!html) return '';
  return html
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/p>/gi, '\n')
    .replace(/<\/div>/gi, '\n')
    .replace(/<\/li>/gi, '\n')
    .replace(/<[^>]*>/g, '')
    .replace(/&amp;/g, '&')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&nbsp;/g, ' ')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function mapStatusCategory(state: string): 'todo' | 'in_progress' | 'done' {
  const lower = state.toLowerCase();
  if (['new', 'to do', 'proposed', 'approved'].includes(lower)) return 'todo';
  if (['closed', 'done', 'resolved', 'completed', 'removed', 'cut'].includes(lower)) return 'done';
  return 'in_progress';
}

/** Parse numeric work item ID from a key like "MyProject-123" */
function parseWorkItemId(issueKey: string): number {
  const parts = issueKey.split('-');
  const num = parseInt(parts[parts.length - 1], 10);
  if (isNaN(num)) throw new Error(`Invalid work item key: ${issueKey}`);
  return num;
}

/** Extract parent key from work item relations */
function extractParentKey(relations: any[] | undefined, projectName: string): string | undefined {
  if (!relations) return undefined;
  const parent = relations.find((r: any) => r.rel === 'System.LinkTypes.Hierarchy-Reverse');
  if (!parent?.url) return undefined;
  // URL looks like: https://dev.azure.com/{org}/_apis/wit/workItems/{id}
  const match = parent.url.match(/\/workItems\/(\d+)$/);
  if (match) return `${projectName}-${match[1]}`;
  return undefined;
}

/** Extract child keys from work item relations */
function extractChildKeys(relations: any[] | undefined, projectName: string): string[] | undefined {
  if (!relations) return undefined;
  const children = relations
    .filter((r: any) => r.rel === 'System.LinkTypes.Hierarchy-Forward')
    .map((r: any) => {
      const match = r.url?.match(/\/workItems\/(\d+)$/);
      return match ? `${projectName}-${match[1]}` : null;
    })
    .filter((k: string | null): k is string => k !== null);
  return children.length > 0 ? children : undefined;
}

function mapWorkItem(account: TrackerAccount, raw: any): TrackerIssue {
  const fields = raw.fields ?? {};
  const projectName = project(account);
  const desc = fields['System.Description'] ?? '';
  return {
    id: String(raw.id),
    key: `${projectName}-${raw.id}`,
    title: fields['System.Title'] ?? '',
    description: stripHtml(desc),
    status: fields['System.State'] ?? 'Unknown',
    statusCategory: mapStatusCategory(fields['System.State'] ?? ''),
    assignee: fields['System.AssignedTo']?.displayName,
    priority: fields['Microsoft.VSTS.Common.Priority'] != null
      ? String(fields['Microsoft.VSTS.Common.Priority'])
      : undefined,
    type: fields['System.WorkItemType'] ?? 'Task',
    labels: fields['System.Tags']
      ? fields['System.Tags'].split(';').map((t: string) => t.trim()).filter(Boolean)
      : [],
    parentKey: extractParentKey(raw.relations, projectName),
    children: extractChildKeys(raw.relations, projectName),
    provider: 'azure-devops',
    accountId: account.id,
    projectKey: projectName,
    url: `https://dev.azure.com/${org(account)}/${projectName}/_workitems/edit/${raw.id}`,
    created: fields['System.CreatedDate'] ?? '',
    updated: fields['System.ChangedDate'] ?? '',
  };
}

function mapPrStatus(status: string): 'open' | 'closed' | 'merged' | 'abandoned' {
  switch (status?.toLowerCase()) {
    case 'active': return 'open';
    case 'completed': return 'merged';
    case 'abandoned': return 'abandoned';
    default: return 'closed';
  }
}

function mapPipelineStatus(result: string | undefined, state: string | undefined): 'running' | 'succeeded' | 'failed' | 'cancelled' | 'queued' {
  if (state?.toLowerCase() === 'inprogress') return 'running';
  if (state?.toLowerCase() === 'canceling') return 'cancelled';
  switch (result?.toLowerCase()) {
    case 'succeeded': return 'succeeded';
    case 'failed': return 'failed';
    case 'canceled': return 'cancelled';
    default: break;
  }
  if (state?.toLowerCase() === 'completed') return 'succeeded';
  return 'queued';
}

// ── WIQL helpers ──

/** Run a WIQL query and return matching work item IDs */
async function wiqlQuery(account: TrackerAccount, token: string, query: string): Promise<number[]> {
  const url = `${baseUrl(account)}/wit/wiql`;
  const data = await adoPost(url, token, { query });
  return (data?.workItems ?? []).map((wi: any) => wi.id as number);
}

/** Batch-fetch work item details by IDs */
async function fetchWorkItems(account: TrackerAccount, token: string, ids: number[]): Promise<any[]> {
  if (ids.length === 0) return [];
  // Azure DevOps limits batch to 200 IDs
  const batches: number[][] = [];
  for (let i = 0; i < ids.length; i += 200) {
    batches.push(ids.slice(i, i + 200));
  }
  const results: any[] = [];
  for (const batch of batches) {
    const idStr = batch.join(',');
    const url = `${baseUrl(account)}/wit/workitems?ids=${idStr}&$expand=relations`;
    const data = await adoGet(url, token);
    results.push(...(data?.value ?? []));
  }
  return results;
}

// ── Config ──

export const azureDevOpsConfigFields: ConfigField[] = [
  {
    key: 'org',
    label: 'Organization',
    placeholder: 'mycompany',
    type: 'text',
    required: true,
  },
  {
    key: 'project',
    label: 'Project',
    placeholder: 'MyProject',
    type: 'text',
    required: true,
  },
];

export const azureDevOpsTokenField: TokenField = {
  label: 'Personal Access Token',
  placeholder: 'Paste your Azure DevOps PAT',
  helpText: 'Generate at https://dev.azure.com/{org}/_usersSettings/tokens',
};

// ── Provider ──

export class AzureDevOpsProvider implements TrackerProvider {
  readonly id = 'azure-devops';
  readonly name = 'Azure DevOps';
  readonly configFields = azureDevOpsConfigFields;
  readonly tokenField = azureDevOpsTokenField;

  async validateCredentials(account: TrackerAccount, token: string): Promise<boolean> {
    try {
      // Use projects endpoint — more reliable than connectionData
      const url = `${orgBaseUrl(account)}/projects`;
      await adoGet(url, token);
      return true;
    } catch (err) {
      console.error('[AzureDevOps] validateCredentials failed:', err);
      return false;
    }
  }

  async listProjects(account: TrackerAccount, token: string): Promise<TrackerProject[]> {
    const url = `${orgBaseUrl(account)}/projects`;
    const data = await adoGet(url, token);
    return (data?.value ?? []).map((p: any) => ({
      id: p.id,
      key: p.name,   // Azure DevOps doesn't have separate keys, use name
      name: p.name,
      avatarUrl: undefined,
      provider: 'azure-devops',
      accountId: account.id,
    }));
  }

  async listIssues(account: TrackerAccount, token: string, options: ListIssuesOptions): Promise<TrackerIssue[]> {
    const clauses: string[] = [];
    const projectName = options.projectKey ?? project(account);
    if (projectName) clauses.push(`[System.TeamProject] = '${projectName}'`);
    if (options.assignedToMe) clauses.push('[System.AssignedTo] = @Me');
    if (options.status) clauses.push(`[System.State] = '${options.status}'`);
    if (options.query) {
      const trimmed = options.query.trim();
      // Check if query looks like a numeric ID or project-id pattern
      const numericMatch = trimmed.match(/^(?:\w+-)?(\d+)$/);
      if (numericMatch) {
        clauses.push(`[System.Id] = ${numericMatch[1]}`);
      } else {
        clauses.push(`[System.Title] CONTAINS '${trimmed.replace(/'/g, "''")}'`);
      }
    }
    if (!options.status && !options.query) {
      // Default: exclude completed items
      clauses.push("[System.State] <> 'Done'");
      clauses.push("[System.State] <> 'Closed'");
      clauses.push("[System.State] <> 'Removed'");
    }

    const where = clauses.length > 0 ? ` WHERE ${clauses.join(' AND ')}` : '';
    const wiql = `SELECT [System.Id] FROM WorkItems${where} ORDER BY [System.ChangedDate] DESC`;

    let ids = await wiqlQuery(account, token, wiql);
    const maxResults = options.maxResults ?? 50;
    const startAt = options.startAt ?? 0;
    ids = ids.slice(startAt, startAt + maxResults);

    const items = await fetchWorkItems(account, token, ids);
    return items.map((raw) => mapWorkItem(account, raw));
  }

  async getIssue(account: TrackerAccount, token: string, issueKey: string): Promise<TrackerIssue | null> {
    try {
      const id = parseWorkItemId(issueKey);
      const url = `${baseUrl(account)}/wit/workitems/${id}?$expand=relations`;
      const raw = await adoGet(url, token);
      return mapWorkItem(account, raw);
    } catch {
      return null;
    }
  }

  async searchIssues(account: TrackerAccount, token: string, query: string): Promise<TrackerIssue[]> {
    return this.listIssues(account, token, { query, maxResults: 20 });
  }

  async listStatuses(_account: TrackerAccount, _token: string, _projectKey: string): Promise<TrackerStatus[]> {
    // Azure DevOps doesn't have a clean per-project status list API.
    // Return common work item states.
    const states = [
      { name: 'New', category: 'todo' as const },
      { name: 'Active', category: 'in_progress' as const },
      { name: 'Resolved', category: 'done' as const },
      { name: 'Closed', category: 'done' as const },
      { name: 'Removed', category: 'done' as const },
    ];
    return states.map((s) => ({
      id: s.name.toLowerCase(),
      name: s.name,
      category: s.category,
    }));
  }

  async getTransitions(account: TrackerAccount, token: string, issueKey: string): Promise<TrackerTransition[]> {
    // First, get the work item to determine its type and current state
    const id = parseWorkItemId(issueKey);
    const itemUrl = `${baseUrl(account)}/wit/workitems/${id}`;
    const item = await adoGet(itemUrl, token);
    const workItemType = item?.fields?.['System.WorkItemType'];
    const currentState = item?.fields?.['System.State'];

    if (!workItemType) return [];

    // Fetch available states for this work item type
    const statesUrl = `${baseUrl(account)}/wit/workitemtypes/${encodeURIComponent(workItemType)}/states`;
    const data = await adoGet(statesUrl, token);
    const states: any[] = data?.value ?? [];

    // Return all states except the current one as transitions
    return states
      .filter((s: any) => s.name !== currentState)
      .map((s: any) => ({
        id: s.name,
        name: `Move to ${s.name}`,
        to: {
          id: s.name.toLowerCase(),
          name: s.name,
          category: mapStatusCategory(s.name),
        },
      }));
  }

  async transitionIssue(account: TrackerAccount, token: string, issueKey: string, transitionId: string): Promise<void> {
    const id = parseWorkItemId(issueKey);
    const url = `${baseUrl(account)}/wit/workitems/${id}`;
    await adoPatch(url, token, [
      { op: 'replace', path: '/fields/System.State', value: transitionId },
    ]);
  }

  async listPullRequests(
    account: TrackerAccount,
    token: string,
    options?: { status?: string; projectKey?: string },
  ): Promise<TrackerPullRequest[]> {
    const projectName = options?.projectKey ?? project(account);
    let url = `https://dev.azure.com/${org(account)}/${projectName}/_apis/git/pullrequests`;
    if (options?.status) {
      url += `?searchCriteria.status=${encodeURIComponent(options.status)}`;
    }
    const data = await adoGet(url, token);
    return (data?.value ?? []).map((pr: any) => ({
      id: String(pr.pullRequestId),
      accountId: account.id,
      provider: 'azure-devops',
      number: pr.pullRequestId,
      title: pr.title ?? '',
      description: pr.description ?? '',
      status: mapPrStatus(pr.status),
      sourceBranch: (pr.sourceRefName ?? '').replace('refs/heads/', ''),
      targetBranch: (pr.targetRefName ?? '').replace('refs/heads/', ''),
      author: pr.createdBy?.displayName ?? '',
      url: `https://dev.azure.com/${org(account)}/${projectName}/_git/${pr.repository?.name ?? ''}/pullrequest/${pr.pullRequestId}`,
      created: pr.creationDate ?? '',
      updated: pr.closedDate ?? pr.creationDate ?? '',
    }));
  }

  async listPipelines(
    account: TrackerAccount,
    token: string,
    options?: { projectKey?: string; maxResults?: number },
  ): Promise<TrackerPipeline[]> {
    const projectName = options?.projectKey ?? project(account);
    const projectBase = `https://dev.azure.com/${org(account)}/${projectName}/_apis`;
    const maxResults = options?.maxResults ?? 20;

    // List pipelines
    const pipelinesUrl = `${projectBase}/pipelines`;
    const pipelinesData = await adoGet(pipelinesUrl, token);
    const pipelines: any[] = pipelinesData?.value ?? [];

    // Fetch recent runs for each pipeline (limit to first 10 pipelines to avoid excessive requests)
    const results: TrackerPipeline[] = [];
    const pipelineSlice = pipelines.slice(0, 10);

    for (const pipeline of pipelineSlice) {
      const runsUrl = `${projectBase}/pipelines/${pipeline.id}/runs`;
      const runsData = await adoGet(runsUrl, token).catch(() => null);
      const runs: any[] = runsData?.value ?? [];

      for (const run of runs.slice(0, 5)) {
        results.push({
          id: String(run.id),
          accountId: account.id,
          provider: 'azure-devops',
          name: run.name ?? pipeline.name ?? '',
          status: mapPipelineStatus(run.result, run.state),
          sourceBranch: (run.resources?.repositories?.self?.refName ?? '').replace('refs/heads/', ''),
          commitSha: run.resources?.repositories?.self?.version ?? '',
          url: run._links?.web?.href ?? `https://dev.azure.com/${org(account)}/${projectName}/_build/results?buildId=${run.id}`,
          startedAt: run.createdDate ?? '',
          finishedAt: run.finishedDate ?? '',
        });
      }

      if (results.length >= maxResults) break;
    }

    return results.slice(0, maxResults);
  }
}
