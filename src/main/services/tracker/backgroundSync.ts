/**
 * Background sync service — periodically syncs issues, PRs, and pipelines
 * from all tracker accounts into SQLite. Also watches git repos for branch
 * changes and auto-links issues to branches/PRs by naming convention.
 */
import { execSync } from 'child_process';
import { issueCache } from '../db';
import { trackerService } from './trackerService';
import { devopsService } from '../devops/devopsService';
import type { TrackerIssue, TrackerPullRequest, TrackerPipeline } from './types';

const ISSUE_KEY_REGEX = /\b[A-Z][A-Z0-9]+-\d+\b/g;

/** Extract issue keys from a string (branch name, PR title, etc.) */
function extractIssueKeys(text: string): string[] {
  const matches = text.match(ISSUE_KEY_REGEX);
  return matches ? [...new Set(matches)] : [];
}

/** Convert a TrackerIssue to the SQLite row format */
function issueToCached(issue: TrackerIssue) {
  return {
    id: issue.id,
    account_id: issue.accountId,
    provider: issue.provider,
    key: issue.key,
    title: issue.title,
    description: issue.description ?? '',
    status: issue.status,
    status_category: issue.statusCategory,
    assignee: issue.assignee ?? null,
    priority: issue.priority ?? null,
    type: issue.type,
    labels: JSON.stringify(issue.labels ?? []),
    project_key: issue.projectKey,
    url: issue.url,
    parent_key: issue.parentKey ?? null,
    created: issue.created,
    updated: issue.updated,
    synced_at: new Date().toISOString(),
  };
}

class BackgroundSyncService {
  private syncInterval: ReturnType<typeof setInterval> | null = null;
  private gitWatchInterval: ReturnType<typeof setInterval> | null = null;
  private watchedRepoPaths = new Set<string>();
  private isSyncing = false;

  /** Start background sync with configurable interval (default 3 min) */
  start(intervalMs = 180_000): void {
    console.log(`[BackgroundSync] starting (interval: ${intervalMs / 1000}s)`);

    // Initial sync after a short delay (let the app boot first)
    setTimeout(() => this.syncAll(), 3000);

    // Periodic sync
    this.syncInterval = setInterval(() => this.syncAll(), intervalMs);

    // Git branch watcher (every 30s)
    this.gitWatchInterval = setInterval(() => this.scanGitBranches(), 30_000);
  }

  stop(): void {
    if (this.syncInterval) { clearInterval(this.syncInterval); this.syncInterval = null; }
    if (this.gitWatchInterval) { clearInterval(this.gitWatchInterval); this.gitWatchInterval = null; }
  }

  /** Register a repo path to watch for branch changes */
  watchRepo(repoPath: string): void {
    this.watchedRepoPaths.add(repoPath);
  }

  /** Run a full sync across all accounts */
  async syncAll(): Promise<void> {
    if (this.isSyncing) return;
    this.isSyncing = true;
    const start = Date.now();

    try {
      // Sync tracker accounts (Jira, etc.)
      const trackerAccounts = trackerService.getAccounts();
      for (const account of trackerAccounts) {
        await this.syncAccount(account.id).catch(err => {
          console.error(`[BackgroundSync] tracker sync failed ${account.label}:`, err?.message);
        });
      }

      // Sync DevOps accounts (Azure DevOps PRs + pipelines)
      const devopsAccounts = devopsService.getAccounts();
      for (const account of devopsAccounts) {
        await this.syncDevOpsAccount(account.id).catch(err => {
          console.error(`[BackgroundSync] devops sync failed ${account.label}:`, err?.message);
        });
      }

      const total = trackerAccounts.length + devopsAccounts.length;
      console.log(`[BackgroundSync] sync complete (${Date.now() - start}ms, ${total} accounts)`);
    } finally {
      this.isSyncing = false;
    }
  }

  /** Sync a single account: issues, PRs, pipelines */
  private async syncAccount(accountId: string): Promise<void> {
    const syncedAt = new Date().toISOString();

    // Sync issues (assigned to me + recent)
    try {
      const issues = await trackerService.listIssues(accountId, { assignedToMe: true, maxResults: 100 });
      const cached = issues.map(issueToCached);
      issueCache.upsertIssues(cached);

      // Create parent-child links
      for (const issue of issues) {
        if (issue.parentKey) {
          issueCache.createIssueLink(issue.parentKey, 'child', issue.key, issue.key);
          issueCache.createIssueLink(issue.key, 'parent', issue.parentKey, issue.parentKey);
        }
      }
    } catch (err: any) {
      console.error(`[BackgroundSync] issues sync failed:`, err?.message);
    }

    // Sync PRs (if provider supports it)
    try {
      const account = trackerService.getAccount(accountId);
      if (!account) return;
      const provider = (trackerService as any).getProvider(account.provider);
      if (provider.listPullRequests) {
        const prs: TrackerPullRequest[] = await provider.listPullRequests(
          account,
          (trackerService as any).retrieveToken(accountId),
          { status: 'active' },
        );
        for (const pr of prs) {
          issueCache.upsertPullRequest({
            id: pr.id,
            account_id: pr.accountId,
            provider: pr.provider,
            number: pr.number,
            title: pr.title,
            description: pr.description,
            status: pr.status,
            source_branch: pr.sourceBranch,
            target_branch: pr.targetBranch,
            author: pr.author,
            url: pr.url,
            created: pr.created,
            updated: pr.updated,
            synced_at: syncedAt,
          });

          // Auto-link PRs to issues by PR title or source branch
          const keysFromTitle = extractIssueKeys(pr.title);
          const keysFromBranch = extractIssueKeys(pr.sourceBranch);
          const allKeys = [...new Set([...keysFromTitle, ...keysFromBranch])];
          for (const key of allKeys) {
            issueCache.createIssueLink(key, 'pr', pr.id, `PR #${pr.number}: ${pr.title.slice(0, 60)}`);
          }
        }
      }
    } catch (err: any) {
      console.error(`[BackgroundSync] PR sync failed:`, err?.message);
    }

    // Sync pipelines (if provider supports it)
    try {
      const account = trackerService.getAccount(accountId);
      if (!account) return;
      const provider = (trackerService as any).getProvider(account.provider);
      if (provider.listPipelines) {
        const pipelines: TrackerPipeline[] = await provider.listPipelines(
          account,
          (trackerService as any).retrieveToken(accountId),
          { maxResults: 20 },
        );
        for (const pl of pipelines) {
          issueCache.upsertPipeline({
            id: pl.id,
            account_id: pl.accountId,
            provider: pl.provider,
            name: pl.name,
            status: pl.status,
            source_branch: pl.sourceBranch,
            commit_sha: pl.commitSha,
            url: pl.url,
            started_at: pl.startedAt,
            finished_at: pl.finishedAt,
            synced_at: syncedAt,
          });

          // Auto-link pipelines to issues by source branch
          const keys = extractIssueKeys(pl.sourceBranch);
          for (const key of keys) {
            issueCache.createIssueLink(key, 'pipeline', pl.id, `${pl.name}: ${pl.status}`);
          }
        }
      }
    } catch (err: any) {
      console.error(`[BackgroundSync] pipeline sync failed:`, err?.message);
    }

    // Cleanup issues older than 7 days
    const cutoff = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString();
    issueCache.deleteStaleIssues(accountId, cutoff);
  }

  /** Scan watched git repos for branches and auto-link to issues */
  /** Sync PRs + pipelines from a DevOps account into SQLite */
  private async syncDevOpsAccount(accountId: string): Promise<void> {
    const syncedAt = new Date().toISOString();

    try {
      const prs = await devopsService.listPullRequests(accountId, { status: 'open' });
      for (const pr of prs) {
        issueCache.upsertPullRequest({
          id: pr.id,
          account_id: pr.accountId,
          provider: pr.provider,
          number: pr.number,
          title: pr.title,
          description: pr.description,
          status: pr.status,
          source_branch: pr.sourceBranch,
          target_branch: pr.targetBranch,
          author: pr.author,
          url: pr.url,
          created: pr.created,
          updated: pr.updated,
          synced_at: syncedAt,
        });
        // Auto-link PRs to issues
        const keys = extractIssueKeys(pr.title + ' ' + pr.sourceBranch);
        for (const key of keys) {
          issueCache.createIssueLink(key, 'pr', pr.id, `PR #${pr.number}: ${pr.title.slice(0, 60)}`);
        }
      }
    } catch (err: any) {
      console.error(`[BackgroundSync] devops PR sync failed:`, err?.message);
    }

    try {
      const pipelines = await devopsService.listPipelines(accountId, { maxResults: 30 });
      for (const pl of pipelines) {
        issueCache.upsertPipeline({
          id: pl.id,
          account_id: pl.accountId,
          provider: pl.provider,
          name: pl.name,
          status: pl.status,
          source_branch: pl.sourceBranch,
          commit_sha: pl.commitSha,
          url: pl.url,
          started_at: pl.startedAt,
          finished_at: pl.finishedAt,
          synced_at: syncedAt,
        });
        const keys = extractIssueKeys(pl.sourceBranch);
        for (const key of keys) {
          issueCache.createIssueLink(key, 'pipeline', pl.id, `${pl.name}: ${pl.status}`);
        }
      }
    } catch (err: any) {
      console.error(`[BackgroundSync] devops pipeline sync failed:`, err?.message);
    }
  }

  private scanGitBranches(): void {
    for (const repoPath of this.watchedRepoPaths) {
      try {
        // Get local branches
        const output = execSync('git branch --format="%(refname:short) %(objectname:short)"', {
          cwd: repoPath,
          encoding: 'utf-8',
          timeout: 5000,
        });

        const now = new Date().toISOString();
        for (const line of output.trim().split('\n')) {
          if (!line.trim()) continue;
          const [name, sha] = line.trim().split(' ');
          issueCache.upsertBranch({
            repo_path: repoPath,
            name,
            is_remote: 0,
            head_sha: sha ?? null,
            updated_at: now,
          });

          // Auto-link branches to issues by name
          const keys = extractIssueKeys(name);
          for (const key of keys) {
            issueCache.createIssueLink(key, 'branch', `${repoPath}:${name}`, name);
          }
        }
      } catch {
        // Git not available or not a repo — skip silently
      }
    }
  }
}

export const backgroundSync = new BackgroundSyncService();
