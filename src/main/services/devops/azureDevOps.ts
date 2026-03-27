/**
 * Azure DevOps provider for git + CI/CD.
 * PRs, pipelines, repos via Azure DevOps REST API.
 */
import type {
  DevOpsProvider,
  DevOpsAccount,
  DevOpsRepo,
  DevOpsPullRequest,
  DevOpsPipeline,
  Reviewer,
  DevOpsConfigField,
  DevOpsTokenField,
} from './types';

// ── Helpers ──

function org(account: DevOpsAccount): string {
  let o = account.config.org ?? '';
  o = o.replace(/^https?:\/\/dev\.azure\.com\/?/i, '').replace(/\/$/, '');
  return o;
}

function project(account: DevOpsAccount): string {
  return account.config.project ?? '';
}

function authHeaders(token: string): Record<string, string> {
  return {
    Authorization: `Basic ${Buffer.from(`:${token}`).toString('base64')}`,
    Accept: 'application/json',
    'Content-Type': 'application/json',
  };
}

function baseUrl(account: DevOpsAccount): string {
  return `https://dev.azure.com/${org(account)}/${project(account)}/_apis`;
}

function orgUrl(account: DevOpsAccount): string {
  return `https://dev.azure.com/${org(account)}/_apis`;
}

async function adoGet(url: string, token: string): Promise<any> {
  const sep = url.includes('?') ? '&' : '?';
  const res = await fetch(`${url}${sep}api-version=7.0`, {
    headers: authHeaders(token),
  });
  if (!res.ok) {
    const body = await res.text().catch(() => '');
    throw new Error(`Azure DevOps ${res.status}: ${body.slice(0, 200)}`);
  }
  return res.json();
}

function mapReviewVote(vote: number): Reviewer['vote'] {
  if (vote === 10) return 'approved';
  if (vote === -10) return 'rejected';
  if (vote === 5) return 'approved'; // approved with suggestions
  if (vote === -5) return 'waiting'; // waiting for author
  return 'no_vote';
}

function mapPrStatus(s: string): DevOpsPullRequest['status'] {
  if (s === 'active') return 'open';
  if (s === 'completed') return 'merged';
  if (s === 'abandoned') return 'abandoned';
  return 'closed';
}

function mapBuildResult(status: string, result: string): DevOpsPipeline['status'] {
  if (status === 'inProgress' || status === 'notStarted') return 'running';
  if (status === 'cancelling') return 'cancelled';
  if (result === 'succeeded') return 'succeeded';
  if (result === 'failed' || result === 'partiallySucceeded') return 'failed';
  if (result === 'canceled') return 'cancelled';
  if (status === 'notStarted') return 'queued';
  return 'running';
}

function stripRef(ref: string): string {
  return ref.replace(/^refs\/heads\//, '');
}

// ── Config ──

export const azureDevOpsConfigFields: DevOpsConfigField[] = [
  { key: 'org', label: 'Organization', placeholder: 'mycompany', type: 'text', required: true },
  { key: 'project', label: 'Project', placeholder: 'MyProject', type: 'text', required: true },
];

export const azureDevOpsTokenField: DevOpsTokenField = {
  label: 'Personal Access Token',
  placeholder: 'Paste your Azure DevOps PAT',
  helpText: 'Generate at https://dev.azure.com/{org}/_usersSettings/tokens',
};

// ── Provider ──

export class AzureDevOpsGitProvider implements DevOpsProvider {
  readonly id = 'azure-devops';
  readonly name = 'Azure DevOps';
  readonly configFields = azureDevOpsConfigFields;
  readonly tokenField = azureDevOpsTokenField;

  async validateCredentials(account: DevOpsAccount, token: string): Promise<boolean> {
    try {
      await adoGet(`${orgUrl(account)}/projects`, token);
      return true;
    } catch (err) {
      console.error('[AzureDevOpsGit] validate failed:', err);
      return false;
    }
  }

  async listRepos(account: DevOpsAccount, token: string): Promise<DevOpsRepo[]> {
    const data = await adoGet(`${baseUrl(account)}/git/repositories`, token);
    return (data.value ?? []).map((r: any) => ({
      id: r.id,
      accountId: account.id,
      provider: 'azure-devops',
      name: r.name,
      defaultBranch: stripRef(r.defaultBranch ?? 'main'),
      url: r.webUrl ?? r.remoteUrl ?? '',
    }));
  }

  async listPullRequests(account: DevOpsAccount, token: string, options?: {
    status?: 'open' | 'all';
    repoId?: string;
  }): Promise<DevOpsPullRequest[]> {
    const status = options?.status === 'all' ? 'all' : 'active';
    let url: string;
    if (options?.repoId) {
      url = `${baseUrl(account)}/git/repositories/${options.repoId}/pullrequests?searchCriteria.status=${status}&$top=50`;
    } else {
      url = `${baseUrl(account)}/git/pullrequests?searchCriteria.status=${status}&$top=50`;
    }

    const data = await adoGet(url, token);
    return (data.value ?? []).map((pr: any) => ({
      id: String(pr.pullRequestId),
      accountId: account.id,
      provider: 'azure-devops',
      number: pr.pullRequestId,
      title: pr.title ?? '',
      description: pr.description ?? '',
      status: mapPrStatus(pr.status),
      sourceBranch: stripRef(pr.sourceRefName ?? ''),
      targetBranch: stripRef(pr.targetRefName ?? ''),
      author: pr.createdBy?.displayName ?? '',
      reviewers: (pr.reviewers ?? []).map((r: any) => ({
        name: r.displayName ?? r.uniqueName ?? '',
        vote: mapReviewVote(r.vote ?? 0),
      })),
      url: `https://dev.azure.com/${org(account)}/${project(account)}/_git/${pr.repository?.name ?? ''}/pullrequest/${pr.pullRequestId}`,
      created: pr.creationDate ?? '',
      updated: pr.closedDate ?? pr.creationDate ?? '',
      isDraft: pr.isDraft ?? false,
      mergeConflicts: pr.mergeStatus === 'conflicts',
    }));
  }

  async listPipelines(account: DevOpsAccount, token: string, options?: {
    maxResults?: number;
  }): Promise<DevOpsPipeline[]> {
    const max = options?.maxResults ?? 30;
    // Get recent builds (more useful than pipeline definitions)
    const data = await adoGet(
      `${baseUrl(account)}/build/builds?$top=${max}&queryOrder=startTimeDescending`,
      token,
    );

    return (data.value ?? []).map((b: any) => {
      const started = b.startTime ?? b.queueTime ?? '';
      const finished = b.finishTime ?? '';
      let duration: number | undefined;
      if (started && finished) {
        duration = Math.round((new Date(finished).getTime() - new Date(started).getTime()) / 1000);
      }

      return {
        id: String(b.id),
        accountId: account.id,
        provider: 'azure-devops',
        name: b.definition?.name ?? `Build ${b.id}`,
        status: mapBuildResult(b.status ?? '', b.result ?? ''),
        sourceBranch: stripRef(b.sourceBranch ?? ''),
        commitSha: (b.sourceVersion ?? '').slice(0, 8),
        author: b.requestedFor?.displayName ?? '',
        url: b._links?.web?.href ?? `https://dev.azure.com/${org(account)}/${project(account)}/_build/results?buildId=${b.id}`,
        startedAt: started,
        finishedAt: finished,
        duration,
      };
    });
  }
}
