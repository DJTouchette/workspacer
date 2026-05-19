/**
 * DevOpsService — thin adapter over devdaemon's normalized /pulls + /pipelines
 * + /repos endpoints. The daemon owns the ADO PAT and rate limiting; this
 * service only translates shapes.
 *
 * One virtual account is exposed (`dd:ado`) when the daemon has ADO auth.
 * Token storage in Workspacer is gone — manage creds via `devdaemon auth ado`.
 */
import * as devdaemon from '../devdaemon/client';
import type {
  DevdaemonPullRequest,
  DevdaemonPipelineRun,
  DevdaemonRepository,
} from '../devdaemon/types';
import type {
  DevOpsAccount,
  DevOpsRepo,
  DevOpsPullRequest,
  DevOpsPipeline,
  DevOpsConfigField,
  DevOpsTokenField,
  Reviewer,
} from './types';

const ACCOUNT_ID = 'dd:ado';

function mapPR(raw: DevdaemonPullRequest): DevOpsPullRequest {
  const status = mapPRStatus(raw.status, raw.is_draft);
  return {
    id: raw.id,
    accountId: ACCOUNT_ID,
    provider: 'ado',
    number: raw.number,
    title: raw.title,
    description: '',
    status,
    sourceBranch: raw.source_branch.replace(/^refs\/heads\//, ''),
    targetBranch: raw.target_branch.replace(/^refs\/heads\//, ''),
    author: raw.author?.display_name ?? '',
    reviewers: (raw.reviewers ?? []).map(
      (r): Reviewer => ({ name: r.display_name, vote: 'no_vote' }),
    ),
    url: raw.url,
    created: raw.created_at,
    updated: raw.updated_at,
    isDraft: raw.is_draft,
    mergeConflicts: false,
  };
}

function mapPRStatus(
  status: DevdaemonPullRequest['status'],
  isDraft: boolean,
): DevOpsPullRequest['status'] {
  if (isDraft) return 'draft';
  if (status === 'active') return 'open';
  if (status === 'completed') return 'merged';
  if (status === 'abandoned') return 'abandoned';
  return 'closed';
}

function mapPipeline(raw: DevdaemonPipelineRun): DevOpsPipeline {
  return {
    id: raw.id,
    accountId: ACCOUNT_ID,
    provider: 'ado',
    name: raw.pipeline_name || `#${raw.run_number}`,
    status: mapPipelineStatus(raw.status, raw.result),
    sourceBranch: raw.branch.replace(/^refs\/heads\//, ''),
    commitSha: raw.commit_id,
    author: '',
    url: raw.url,
    startedAt: raw.started_at,
    finishedAt: raw.finished_at ?? '',
  };
}

function mapPipelineStatus(
  status: string,
  result: string,
): DevOpsPipeline['status'] {
  if (status === 'inProgress' || status === 'running') return 'running';
  if (status === 'queued' || status === 'notStarted') return 'queued';
  if (status === 'cancelling') return 'running';
  if (result === 'succeeded') return 'succeeded';
  if (result === 'failed') return 'failed';
  if (result === 'canceled' || result === 'cancelled') return 'cancelled';
  return 'queued';
}

function mapRepo(raw: DevdaemonRepository): DevOpsRepo {
  return {
    id: raw.id,
    accountId: ACCOUNT_ID,
    provider: 'ado',
    name: raw.project ? `${raw.project}/${raw.name}` : raw.name,
    defaultBranch: (raw.default_branch ?? '').replace(/^refs\/heads\//, ''),
    url: raw.url,
  };
}

class DevOpsService {
  async getProviderList(): Promise<
    Array<{ id: string; name: string; configFields: DevOpsConfigField[]; tokenField: DevOpsTokenField }>
  > {
    return [
      {
        id: 'ado',
        name: 'Azure DevOps',
        configFields: [],
        tokenField: {
          label: 'Credentials managed by devdaemon',
          placeholder: '',
          helpText: 'Run `devdaemon auth ado --pat <token>`',
        },
      },
    ];
  }

  async getAccounts(): Promise<DevOpsAccount[]> {
    try {
      const status = await devdaemon.getAuthStatus();
      if (status.ado !== 'ok') return [];
    } catch {
      return [];
    }
    return [
      {
        id: ACCOUNT_ID,
        provider: 'ado',
        label: 'Azure DevOps (devdaemon)',
        config: {},
      },
    ];
  }

  async addAccount(
    _provider: string,
    _label: string,
    _config: Record<string, string>,
    _token: string,
  ): Promise<DevOpsAccount> {
    throw new Error('DevOps accounts are managed by devdaemon. Run `devdaemon auth ado --pat <token>`.');
  }

  removeAccount(_accountId: string): void {
    throw new Error('DevOps accounts are managed by devdaemon.');
  }

  async listRepos(_accountId: string): Promise<DevOpsRepo[]> {
    const repos = await devdaemon.listRepositories();
    return repos.map(mapRepo);
  }

  async listPullRequests(
    _accountId: string,
    options?: { status?: 'open' | 'all'; repoId?: string },
  ): Promise<DevOpsPullRequest[]> {
    const status = options?.status === 'all' ? undefined : 'active';
    const prs = await devdaemon.listPullRequests({ status });
    if (options?.repoId) {
      return prs.filter((p) => p.repository === options.repoId).map(mapPR);
    }
    return prs.map(mapPR);
  }

  async listPipelines(
    _accountId: string,
    _options?: { maxResults?: number },
  ): Promise<DevOpsPipeline[]> {
    const runs = await devdaemon.listPipelines();
    return runs.map(mapPipeline);
  }
}

export const devopsService = new DevOpsService();
