import type BetterSqlite3 from 'better-sqlite3';
import { DatabaseService, database } from './database';

// ── Row types for the cache layer ──

export interface CachedIssue {
  id: string;
  account_id: string;
  provider: string;
  key: string;
  title: string;
  description: string;
  status: string;
  status_category: string;
  assignee: string | null;
  priority: string | null;
  type: string;
  labels: string; // JSON array
  project_key: string;
  url: string;
  parent_key: string | null;
  created: string;
  updated: string;
  synced_at: string;
}

export interface CachedBranch {
  id?: number;
  repo_path: string;
  name: string;
  is_remote: number;
  head_sha: string | null;
  updated_at: string;
}

export interface CachedPullRequest {
  id: string;
  account_id: string;
  provider: string;
  number: number | null;
  title: string;
  description: string;
  status: string;
  source_branch: string;
  target_branch: string;
  author: string | null;
  url: string;
  created: string;
  updated: string;
  synced_at: string;
}

export interface CachedPipeline {
  id: string;
  account_id: string;
  provider: string;
  name: string;
  status: string;
  source_branch: string;
  commit_sha: string | null;
  url: string;
  started_at: string | null;
  finished_at: string | null;
  synced_at: string;
}

export interface CachedIssueLink {
  id?: number;
  issue_key: string;
  link_type: string;
  link_id: string;
  link_label: string;
  created_at: string;
}

export interface IssueFilterOptions {
  projectKey?: string;
  status?: string;
  statusCategory?: string;
  assignee?: string;
  limit?: number;
  offset?: number;
}

export class IssueCache {
  private dbService: DatabaseService;

  // Prepared statements (lazily initialized)
  private _stmts: Record<string, BetterSqlite3.Statement> | null = null;

  constructor(dbService: DatabaseService) {
    this.dbService = dbService;
  }

  private get db(): BetterSqlite3.Database {
    return this.dbService.db;
  }

  /** Lazily prepare and cache all statements. */
  private get stmts(): Record<string, BetterSqlite3.Statement> {
    if (!this._stmts) {
      this._stmts = {
        upsertIssue: this.db.prepare(`
          INSERT INTO issues (id, account_id, provider, key, title, description, status, status_category,
            assignee, priority, type, labels, project_key, url, parent_key, created, updated, synced_at)
          VALUES (@id, @account_id, @provider, @key, @title, @description, @status, @status_category,
            @assignee, @priority, @type, @labels, @project_key, @url, @parent_key, @created, @updated, @synced_at)
          ON CONFLICT(account_id, key) DO UPDATE SET
            id=excluded.id, provider=excluded.provider, title=excluded.title,
            description=excluded.description, status=excluded.status,
            status_category=excluded.status_category, assignee=excluded.assignee,
            priority=excluded.priority, type=excluded.type, labels=excluded.labels,
            project_key=excluded.project_key, url=excluded.url, parent_key=excluded.parent_key,
            created=excluded.created, updated=excluded.updated, synced_at=excluded.synced_at
        `),

        getIssueByKey: this.db.prepare(`SELECT * FROM issues WHERE key = ?`),

        getIssuesByAccount: this.db.prepare(`SELECT * FROM issues WHERE account_id = ? ORDER BY updated DESC`),

        getChildIssues: this.db.prepare(`SELECT * FROM issues WHERE parent_key = ? ORDER BY key`),

        getIssueLinks: this.db.prepare(`SELECT * FROM issue_links WHERE issue_key = ?`),

        upsertBranch: this.db.prepare(`
          INSERT INTO branches (repo_path, name, is_remote, head_sha, updated_at)
          VALUES (@repo_path, @name, @is_remote, @head_sha, @updated_at)
          ON CONFLICT(repo_path, name, is_remote) DO UPDATE SET
            head_sha=excluded.head_sha, updated_at=excluded.updated_at
        `),

        upsertPullRequest: this.db.prepare(`
          INSERT INTO pull_requests (id, account_id, provider, number, title, description, status,
            source_branch, target_branch, author, url, created, updated, synced_at)
          VALUES (@id, @account_id, @provider, @number, @title, @description, @status,
            @source_branch, @target_branch, @author, @url, @created, @updated, @synced_at)
          ON CONFLICT(id) DO UPDATE SET
            account_id=excluded.account_id, provider=excluded.provider, number=excluded.number,
            title=excluded.title, description=excluded.description, status=excluded.status,
            source_branch=excluded.source_branch, target_branch=excluded.target_branch,
            author=excluded.author, url=excluded.url, created=excluded.created,
            updated=excluded.updated, synced_at=excluded.synced_at
        `),

        upsertPipeline: this.db.prepare(`
          INSERT INTO pipelines (id, account_id, provider, name, status, source_branch,
            commit_sha, url, started_at, finished_at, synced_at)
          VALUES (@id, @account_id, @provider, @name, @status, @source_branch,
            @commit_sha, @url, @started_at, @finished_at, @synced_at)
          ON CONFLICT(id) DO UPDATE SET
            account_id=excluded.account_id, provider=excluded.provider, name=excluded.name,
            status=excluded.status, source_branch=excluded.source_branch,
            commit_sha=excluded.commit_sha, url=excluded.url, started_at=excluded.started_at,
            finished_at=excluded.finished_at, synced_at=excluded.synced_at
        `),

        createIssueLink: this.db.prepare(`
          INSERT OR IGNORE INTO issue_links (issue_key, link_type, link_id, link_label, created_at)
          VALUES (?, ?, ?, ?, ?)
        `),

        deleteStaleIssues: this.db.prepare(`
          DELETE FROM issues WHERE account_id = ? AND synced_at < ?
        `),

        searchIssues: this.db.prepare(`
          SELECT * FROM issues WHERE key LIKE ? OR title LIKE ? ORDER BY updated DESC LIMIT 50
        `),
      };
    }
    return this._stmts;
  }

  // ── Issue methods ──

  upsertIssue(issue: CachedIssue): void {
    this.stmts.upsertIssue.run(issue);
  }

  upsertIssues(issues: CachedIssue[]): void {
    const tx = this.db.transaction((rows: CachedIssue[]) => {
      for (const issue of rows) {
        this.stmts.upsertIssue.run(issue);
      }
    });
    tx(issues);
  }

  getIssueByKey(key: string): CachedIssue | undefined {
    return this.stmts.getIssueByKey.get(key) as CachedIssue | undefined;
  }

  getIssuesByAccount(accountId: string, options?: IssueFilterOptions): CachedIssue[] {
    // If no filters, use the simple prepared statement
    if (!options || (!options.projectKey && !options.status && !options.statusCategory && !options.assignee)) {
      const rows = this.stmts.getIssuesByAccount.all(accountId) as CachedIssue[];
      if (options?.limit) {
        const offset = options.offset ?? 0;
        return rows.slice(offset, offset + options.limit);
      }
      return rows;
    }

    // Build a filtered query dynamically
    const conditions: string[] = ['account_id = ?'];
    const params: (string | number)[] = [accountId];

    if (options.projectKey) {
      conditions.push('project_key = ?');
      params.push(options.projectKey);
    }
    if (options.status) {
      conditions.push('status = ?');
      params.push(options.status);
    }
    if (options.statusCategory) {
      conditions.push('status_category = ?');
      params.push(options.statusCategory);
    }
    if (options.assignee) {
      conditions.push('assignee = ?');
      params.push(options.assignee);
    }

    let sql = `SELECT * FROM issues WHERE ${conditions.join(' AND ')} ORDER BY updated DESC`;

    if (options.limit) {
      sql += ` LIMIT ?`;
      params.push(options.limit);
      if (options.offset) {
        sql += ` OFFSET ?`;
        params.push(options.offset);
      }
    }

    return this.db.prepare(sql).all(...params) as CachedIssue[];
  }

  getChildIssues(parentKey: string): CachedIssue[] {
    return this.stmts.getChildIssues.all(parentKey) as CachedIssue[];
  }

  getIssueLinks(issueKey: string): CachedIssueLink[] {
    return this.stmts.getIssueLinks.all(issueKey) as CachedIssueLink[];
  }

  // ── Branch methods ──

  upsertBranch(branch: CachedBranch): void {
    this.stmts.upsertBranch.run(branch);
  }

  // ── Pull request methods ──

  upsertPullRequest(pr: CachedPullRequest): void {
    this.stmts.upsertPullRequest.run(pr);
  }

  // ── Pipeline methods ──

  upsertPipeline(pipeline: CachedPipeline): void {
    this.stmts.upsertPipeline.run(pipeline);
  }

  // ── Link methods ──

  createIssueLink(issueKey: string, linkType: string, linkId: string, label: string): void {
    this.stmts.createIssueLink.run(issueKey, linkType, linkId, label, new Date().toISOString());
  }

  // ── Cleanup ──

  deleteStaleIssues(accountId: string, olderThan: string): number {
    const result = this.stmts.deleteStaleIssues.run(accountId, olderThan);
    return result.changes;
  }

  // ── Search ──

  searchIssues(query: string): CachedIssue[] {
    const pattern = `%${query}%`;
    return this.stmts.searchIssues.all(pattern, pattern) as CachedIssue[];
  }

  // ── Pipeline + PR queries ──

  getRecentPipelines(limit = 20): CachedPipeline[] {
    return this.db.prepare(
      'SELECT * FROM pipelines ORDER BY started_at DESC LIMIT ?'
    ).all(limit) as CachedPipeline[];
  }

  getRecentPullRequests(limit = 20): CachedPullRequest[] {
    return this.db.prepare(
      'SELECT * FROM pull_requests ORDER BY updated DESC LIMIT ?'
    ).all(limit) as CachedPullRequest[];
  }
}

export const issueCache = new IssueCache(database);
