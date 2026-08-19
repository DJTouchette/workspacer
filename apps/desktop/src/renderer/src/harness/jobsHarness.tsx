/**
 * Standalone Jobs harness — Settings → Jobs against a STATEFUL in-memory fake
 * of the hub's jobs.* RPCs, with no Electron and no live hub. Fully
 * interactive: add/edit/toggle/delete jobs, hit Run now (runs "finish" after a
 * moment, alternating ok/failed so both chips appear), expand run history.
 *
 * Seeded with one job per interesting state: a healthy daily agent, a failing
 * shell job, a skipped-overlap job, one currently running, one disabled, and a
 * manual one that has never run. The empty state (template chips) is reachable
 * by deleting everything — or open with ?empty.
 *
 * Open http://localhost:5173/jobs-harness.html with the dev server running.
 */
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import type { HubJob, HubJobRun, HubJobView } from '../../../main/shared/ipcTypes';

const now = Date.now();
const min = 60_000;

const run = (
  jobId: string,
  startAgoMin: number,
  status: HubJobRun['status'],
  detail: string,
  durSec = 40,
): HubJobRun => ({
  jobId,
  startedAt: now - startAgoMin * min,
  finishedAt: now - startAgoMin * min + durSec * 1000,
  status,
  detail,
});

const jobs = new Map<string, HubJobView>();
const history = new Map<string, HubJobRun[]>();
const running = new Set<string>();

function seed(): void {
  const rows: Array<{ j: HubJobView; runs: HubJobRun[] }> = [
    {
      j: {
        id: 'j-triage',
        name: 'Morning triage',
        enabled: true,
        trigger: { kind: 'daily', at: '09:00', days: [1, 2, 3, 4, 5] },
        action: {
          kind: 'spawn',
          spawn: {
            cwd: '/home/you/work/api-gateway',
            prompt:
              'Triage anything that happened overnight — new issues, failed CI, stale branches — and finish with a short summary.',
            provider: 'claude',
          },
        },
        nextRunAt: now + 16 * 60 * min,
      },
      runs: [
        run('j-triage', 60 * 8, 'ok', 'spawned 0d1f6c2e-4b7a-4d2c-9e1f-aaa111bbb222', 340),
        run('j-triage', 60 * 32, 'ok', 'spawned 77c3a9d0-1e2f-4a5b-8c9d-ccc333ddd444', 298),
        run('j-triage', 60 * 56, 'ok', 'spawned 4b8e2f1a-9c0d-4e6f-a1b2-eee555fff666', 401),
      ],
    },
    {
      j: {
        id: 'j-backup',
        name: 'Nightly transcript backup',
        enabled: true,
        trigger: { kind: 'daily', at: '02:30' },
        action: {
          kind: 'shell',
          shell: { command: 'rsync -a ~/.claude/projects/ /backup/claude/', cwd: '' },
        },
        nextRunAt: now + 7 * 60 * min,
      },
      runs: [
        run(
          'j-backup',
          60 * 16,
          'error',
          'exit status 23 — output: rsync: mkdir "/backup/claude" failed: No such file or directory (2)',
          3,
        ),
        run('j-backup', 60 * 40, 'ok', 'sent 1,204 files  4.2MB/s', 41),
      ],
    },
    {
      j: {
        id: 'j-warm',
        name: 'Keep models warm',
        enabled: true,
        trigger: { kind: 'interval', everyMinutes: 30 },
        action: { kind: 'call', call: { method: 'notifications.post' } },
        nextRunAt: now + 12 * min,
      },
      runs: [
        run('j-warm', 18, 'skipped', 'previous run still in progress', 0),
        run('j-warm', 48, 'ok', '{"ok":true}', 2),
        run('j-warm', 78, 'ok', '{"ok":true}', 2),
      ],
    },
    {
      j: {
        id: 'j-report',
        name: 'Weekly usage report',
        enabled: true,
        trigger: { kind: 'daily', at: '17:00', days: [5] },
        action: {
          kind: 'spawn',
          spawn: {
            cwd: '/home/you/work/workspacer',
            prompt: 'Summarize this week: commits, costs, agents run. Write REPORT.md.',
            provider: 'claude',
          },
        },
        nextRunAt: now + 3 * 24 * 60 * min,
        running: true,
      },
      runs: [run('j-report', 60 * 24 * 7, 'ok', 'spawned 8a1b2c3d-...', 512)],
    },
    {
      j: {
        id: 'j-cleanup',
        name: 'Prune worktrees',
        enabled: false,
        trigger: { kind: 'interval', everyMinutes: 240 },
        action: { kind: 'shell', shell: { command: 'git worktree prune', cwd: '/home/you/work' } },
      },
      runs: [run('j-cleanup', 60 * 50, 'ok', '', 1)],
    },
    {
      j: {
        id: 'j-release',
        name: 'Cut a release build',
        enabled: true,
        trigger: { kind: 'manual' },
        action: {
          kind: 'shell',
          shell: { command: 'gh workflow run release.yml -f nightly=true' },
        },
      },
      runs: [],
    },
  ];
  for (const { j, runs } of rows) {
    jobs.set(j.id, j);
    history.set(j.id, runs);
    if (j.running) running.add(j.id);
  }
}
if (!new URLSearchParams(location.search).has('empty')) seed();

let fakeRunFlip = false;

(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    jobsList: async (): Promise<{ jobs: HubJobView[] }> => ({
      jobs: [...jobs.values()].map((j) => ({
        ...j,
        running: running.has(j.id),
        lastRun: history.get(j.id)?.[0],
      })),
    }),
    jobsUpsert: async (job: HubJob): Promise<HubJob> => {
      const id = job.id || `j-${Math.random().toString(36).slice(2, 8)}`;
      const prev = jobs.get(id);
      const next: HubJobView = {
        ...job,
        id,
        nextRunAt:
          job.enabled && job.trigger.kind !== 'manual' ? now + 45 * min : undefined,
        lastRun: prev?.lastRun,
      };
      jobs.set(id, next);
      if (!history.has(id)) history.set(id, []);
      return next;
    },
    jobsRemove: async (id: string) => {
      jobs.delete(id);
      history.delete(id);
      running.delete(id);
      return { ok: true };
    },
    jobsRun: async (id: string) => {
      if (running.has(id)) return { started: false, reason: 'already running' };
      running.add(id);
      // "Finish" after a few seconds, alternating success/failure so both
      // result chips show up while you're clicking around.
      setTimeout(() => {
        running.delete(id);
        fakeRunFlip = !fakeRunFlip;
        const r: HubJobRun = fakeRunFlip
          ? {
              jobId: id,
              startedAt: Date.now() - 4000,
              finishedAt: Date.now(),
              status: 'ok',
              detail: 'spawned 5e6f7a8b-run-now',
            }
          : {
              jobId: id,
              startedAt: Date.now() - 4000,
              finishedAt: Date.now(),
              status: 'error',
              detail: 'agents.spawn: no provider for method',
            };
        history.set(id, [r, ...(history.get(id) ?? [])]);
      }, 4000);
      return { started: true };
    },
    jobsHistory: async (id: string) => ({ runs: history.get(id) ?? [] }),
    pickFolder: async () => '/home/you/work/picked-directory',
  },
  {
    get(target: any, prop: string) {
      if (prop in target) return target[prop];
      if (typeof prop === 'string' && prop.startsWith('on')) {
        return () => () => {};
      }
      return () => Promise.resolve(undefined);
    },
  },
);

// Deferred imports so the stub is installed first.
const { default: JobsSection } = await import('../components/settings/JobsSection');
const { resolveTheme, applyTheme } = await import('../themes');

applyTheme(resolveTheme(new URLSearchParams(location.search).get('theme') ?? 'dark'));

const Frame: React.FC = () => (
  <div
    style={{
      minHeight: '100vh',
      background: 'var(--wks-bg-base)',
      display: 'flex',
      justifyContent: 'center',
      padding: '32px 16px',
      fontFamily: 'var(--wks-font-sans)',
    }}
  >
    <div style={{ width: 640, maxWidth: '100%' }}>
      <JobsSection />
    </div>
  </div>
);

createRoot(document.getElementById('root')!).render(<Frame />);
