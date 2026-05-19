/**
 * Pipe client for devdaemon. Talks to the daemon over the Windows named pipe
 * (or POSIX unix socket), which bypasses bearer-token auth — OS ACL handles
 * access control.
 *
 * Source of truth for endpoints lives in
 *   C:\Users\DamienTouchette\work\biai\internal\daemon\daemon.go
 */
import * as http from 'http';
import * as os from 'os';

import type {
  DevdaemonHealth,
  DevdaemonAuthStatus,
  DevdaemonIssue,
  DevdaemonProject,
  DevdaemonRepository,
  DevdaemonTransition,
  DevdaemonPullRequest,
  DevdaemonPipelineRun,
  DevdaemonUser,
  DevdaemonEvent,
} from './types';

// Windows named pipe / Unix socket — must match
// internal/paths/paths_{windows,unix}.go on the daemon side.
const PIPE_PATH =
  os.platform() === 'win32'
    ? '\\\\.\\pipe\\devdaemon'
    : (() => {
        const xdg = process.env.XDG_RUNTIME_DIR;
        if (xdg) return `${xdg}/devdaemon.sock`;
        return `${os.homedir()}/.devdaemon.sock`;
      })();

interface RequestOptions {
  method?: 'GET' | 'POST' | 'DELETE';
  path: string;
  body?: unknown;
}

function request<T>(opts: RequestOptions): Promise<T> {
  const { method = 'GET', path, body } = opts;
  const payload = body === undefined ? undefined : JSON.stringify(body);

  return new Promise<T>((resolve, reject) => {
    const req = http.request(
      {
        socketPath: PIPE_PATH,
        method,
        path,
        headers: {
          Accept: 'application/json',
          ...(payload ? { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) } : {}),
        },
      },
      (res) => {
        const chunks: Buffer[] = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => {
          const buf = Buffer.concat(chunks);
          const status = res.statusCode ?? 0;
          if (status === 204 || buf.length === 0) {
            resolve(undefined as T);
            return;
          }
          let parsed: unknown;
          try {
            parsed = JSON.parse(buf.toString('utf8'));
          } catch (err) {
            reject(new Error(`devdaemon ${path}: invalid JSON (status ${status}): ${buf.toString('utf8').slice(0, 200)}`));
            return;
          }
          if (status >= 400) {
            const msg = (parsed as { error?: string })?.error ?? `status ${status}`;
            reject(new Error(`devdaemon ${path}: ${msg}`));
            return;
          }
          resolve(parsed as T);
        });
      },
    );
    req.on('error', reject);
    if (payload) req.write(payload);
    req.end();
  });
}

// ── Health & auth ──

export const getHealth = () => request<DevdaemonHealth>({ path: '/healthz' });
export const getAuthStatus = () => request<DevdaemonAuthStatus>({ path: '/auth/status' });

// ── Issues ──

export interface SearchIssuesOptions {
  sources?: Array<'jira' | 'ado'>;
  query?: string;
  assignee?: string;        // "me" or a username
  status?: string;
}

export function searchIssues(opts: SearchIssuesOptions = {}): Promise<DevdaemonIssue[]> {
  const params = new URLSearchParams();
  if (opts.sources?.length) params.set('source', opts.sources.join(','));
  if (opts.query) params.set('q', opts.query);
  if (opts.assignee) params.set('assignee', opts.assignee);
  if (opts.status) params.set('status', opts.status);
  const qs = params.toString();
  return request<DevdaemonIssue[]>({ path: `/issues${qs ? `?${qs}` : ''}` });
}

export const getIssue = (id: string) =>
  request<DevdaemonIssue>({ path: `/issues/${encodeURIComponent(id)}` });

export const getTransitions = (id: string) =>
  request<DevdaemonTransition[]>({ path: `/issues/${encodeURIComponent(id)}/transitions` });

export const applyTransition = (id: string, transitionId: string) =>
  request<void>({
    method: 'POST',
    path: `/issues/${encodeURIComponent(id)}/transitions`,
    body: { transition_id: transitionId },
  });

// ── Projects / PRs / Pipelines ──

export function listProjects(source?: 'jira' | 'ado'): Promise<DevdaemonProject[]> {
  const path = source ? `/projects?source=${source}` : '/projects';
  return request<DevdaemonProject[]>({ path });
}

export function listRepositories(project?: string): Promise<DevdaemonRepository[]> {
  const path = project ? `/repos?project=${encodeURIComponent(project)}` : '/repos';
  return request<DevdaemonRepository[]>({ path });
}

export interface ListPullsOptions {
  project?: string;
  repo?: string;
  status?: 'active' | 'completed' | 'abandoned';
}

export function listPullRequests(opts: ListPullsOptions = {}): Promise<DevdaemonPullRequest[]> {
  const params = new URLSearchParams();
  if (opts.project) params.set('project', opts.project);
  if (opts.repo) params.set('repo', opts.repo);
  if (opts.status) params.set('status', opts.status);
  const qs = params.toString();
  return request<DevdaemonPullRequest[]>({ path: `/pulls${qs ? `?${qs}` : ''}` });
}

export interface ListPipelinesOptions {
  project?: string;
  pipelineId?: number;
}

export function listPipelines(opts: ListPipelinesOptions = {}): Promise<DevdaemonPipelineRun[]> {
  const params = new URLSearchParams();
  if (opts.project) params.set('project', opts.project);
  if (opts.pipelineId !== undefined) params.set('pipeline_id', String(opts.pipelineId));
  const qs = params.toString();
  return request<DevdaemonPipelineRun[]>({ path: `/pipelines${qs ? `?${qs}` : ''}` });
}

// ── Me ──

export const getMe = () => request<{ jira?: DevdaemonUser; ado?: DevdaemonUser }>({ path: '/users/me' });

// ── Events (SSE) ──

export interface EventSubscription {
  close(): void;
}

/**
 * Subscribe to the daemon's `/events` SSE stream. Emits each event as it
 * arrives; reconnects on error after `reconnectMs`. Returns a handle to
 * tear down the subscription.
 */
export function subscribeEvents(onEvent: (e: DevdaemonEvent) => void, reconnectMs = 2000): EventSubscription {
  let req: http.ClientRequest | null = null;
  let stopped = false;

  const connect = () => {
    if (stopped) return;
    req = http.request(
      {
        socketPath: PIPE_PATH,
        method: 'GET',
        path: '/events',
        headers: { Accept: 'text/event-stream' },
      },
      (res) => {
        let buffer = '';
        res.setEncoding('utf8');
        res.on('data', (chunk: string) => {
          buffer += chunk;
          let idx: number;
          // SSE events are separated by blank lines (\n\n).
          while ((idx = buffer.indexOf('\n\n')) !== -1) {
            const frame = buffer.slice(0, idx);
            buffer = buffer.slice(idx + 2);
            const data = frame
              .split('\n')
              .filter((l) => l.startsWith('data:'))
              .map((l) => l.slice(5).trimStart())
              .join('\n');
            if (!data) continue;
            try {
              onEvent(JSON.parse(data) as DevdaemonEvent);
            } catch {
              // Ignore malformed frames — log already happened upstream.
            }
          }
        });
        res.on('end', () => {
          if (!stopped) setTimeout(connect, reconnectMs);
        });
        res.on('error', () => {
          if (!stopped) setTimeout(connect, reconnectMs);
        });
      },
    );
    req.on('error', () => {
      if (!stopped) setTimeout(connect, reconnectMs);
    });
    req.end();
  };

  connect();

  return {
    close() {
      stopped = true;
      req?.destroy();
    },
  };
}
