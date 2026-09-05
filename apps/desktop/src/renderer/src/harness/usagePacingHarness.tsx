/** Isolated visual fixture: real account cards, no daemon, credentials or probing. */
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import { applyTheme, darkTheme, lightTheme } from '../themes';
import { UsageReportCard } from '../components/UsageReportCard';
import { InspectorCard } from '../components/claude/InspectorCard';
import UsageScheduleRow from '../components/settings/UsageScheduleRow';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import type {
  UsagePacingSchedule,
  UsagePacingScheduleWire,
  UsageReportWire,
} from '../../../main/shared/usageReport';
const params = new URLSearchParams(location.search);
applyTheme(params.get('theme') === 'light' ? lightTheme : darkTheme);

// The Settings control that chooses WHICH week the cards above are paced
// against. It reads window.electronAPI, so the fixture supplies one — a hub
// that answers (?schedule=five_day|seven_day|unset, the default) or one that
// cannot (?schedule=unavailable), which is the state an older server produces
// and the one worth looking at with your own eyes.
const requested = params.get('schedule') ?? 'five_day';
let stored: UsagePacingScheduleWire | null =
  requested === 'unavailable'
    ? null
    : {
        schedule: requested === 'unset' ? '' : (requested as UsagePacingSchedule),
        configurable: true,
      };
(window as unknown as { electronAPI: unknown }).electronAPI = {
  usageReport: async () => inspectorReport,
  usagePacingSchedule: async () => stored,
  setUsagePacingSchedule: async (schedule: UsagePacingSchedule) => {
    stored = { schedule, configurable: true };
    return { ok: true as const, state: stored };
  },
};
const now = Math.floor(Date.now() / 1000);
const report: UsageReportWire = { evaluated_at: now, valid_until: now + 60 };
const accounts = [
  { account: '', label: 'Default', pct: 0, expected: 52 },
  {
    account: '/home/user/company/department/team/accounts/work',
    label: 'work',
    pct: 70,
    expected: 52,
  },
  { account: '/home/user/personal/accounts/work', label: 'work', pct: 52, expected: 52 },
  { account: null, label: 'Unattributed', pct: 14, expected: undefined },
];

/**
 * The Inspector's Usage tab, whose account block reads the SAME report through
 * `window.electronAPI.usageReport` above — the shared hook, not a second fetch.
 * Rendered at rail width, which is where clipping shows up: this column is
 * ~320px on a 1200px window and full width on a phone-sized one.
 */
const AGED_ROOT = '/home/user/.claude/accounts/aged';
const BLIND_ROOT = '/home/user/.claude/accounts/blind';
const inspectorReport: UsageReportWire = {
  evaluated_at: now,
  valid_until: now + 60,
  providers: [
    {
      provider: 'claude',
      accounts: [
        { account: '', label: 'Default', pct: 12, expected: 52 },
        { account: '/home/user/.claude/accounts/work', label: 'work', pct: 70, expected: 52 },
        // An observation the provider has not refreshed: the percentage stays,
        // the tick goes, and the card has to say it is history.
        { account: AGED_ROOT, label: 'aged', pct: 64, expected: 52, fresh: false },
        // A window that is running but whose measurement did not arrive. The
        // row that must never read "0% used" or its mirror "100% left".
        { account: BLIND_ROOT, label: 'blind', pct: null, expected: 52 },
      ].map((a) => ({
        account: a.account,
        label: a.label,
        source: 'disk',
        observed_at: now - 300,
        fresh: a.fresh ?? null,
        windows: {
          five_hour: {
            used_percent: { state: 'ok' as const, value: a.pct },
            resets_at: now + 9000,
            window_minutes: 300,
            is_current: true,
            pace: {
              known: true,
              state: 'ahead',
              usedPct: a.pct ?? undefined,
              expectedPct: a.expected,
              curve: 'calendar',
            },
          },
          seven_day: {
            used_percent: { state: 'ok' as const, value: a.pct === null ? null : a.pct + 19 },
            // More than a day out, so this row reads as a local wall clock
            // rather than as a countdown.
            resets_at: now + 3 * 86400,
            window_minutes: 10080,
            is_current: true,
            pace: {
              known: true,
              state: 'ahead',
              usedPct: a.pct === null ? undefined : a.pct + 19,
              expectedPct: 57,
              curve: 'five_day',
            },
          },
        },
      })),
    },
  ],
};

const inspectorSnapshot = (overrides: Partial<ClaudeSessionSnapshot>): ClaudeSessionSnapshot =>
  ({
    sessionId: 'sess-1',
    cwd: '/home/user/Work/workspacer',
    ptyId: 'sess-1',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    pendingQuestions: null,
    subagents: [],
    workflows: [],
    ambientState: 'idle',
    lastActivity: Date.now(),
    totalToolCalls: 12,
    provider: 'claude',
    usage: {
      model: 'claude-opus-5',
      contextTokens: 96_000,
      contextLimit: 200_000,
      totalInputTokens: 412_000,
      totalOutputTokens: 38_400,
      costUSD: 4.17,
    },
    statusLine: {
      fiveHourPct: 70,
      fiveHourResetsAt: now + 9000,
      fiveHourWindowMins: 300,
      receivedAt: new Date().toISOString(),
    },
    ...overrides,
  }) as unknown as ClaudeSessionSnapshot;

/** One column per state the block can reach: the identified account, the
 *  two-login case it refuses to guess at, a session on a peer hub, an
 *  observation the provider has aged, and a running window whose measurement
 *  never arrived. The last two are the ones worth looking at with your own
 *  eyes: neither may read as a fresh figure, and neither may imply a full
 *  allowance. */
const inspectors: Array<{ title: string; snapshot: ClaudeSessionSnapshot }> = [
  {
    title: 'Identified account · above pace',
    snapshot: inspectorSnapshot({
      transcriptPath: '/home/user/.claude/accounts/work/projects/p/t.jsonl',
    }),
  },
  {
    title: 'Two logins, session names neither',
    snapshot: inspectorSnapshot({ sessionId: 'sess-2', transcriptPath: undefined }),
  },
  {
    title: 'Session on a peer hub',
    snapshot: inspectorSnapshot({ sessionId: 'sess-3', transcriptPath: '', hub: 'studio' }),
  },
  {
    title: 'Stale provider observation',
    snapshot: inspectorSnapshot({
      sessionId: 'sess-4',
      transcriptPath: `${AGED_ROOT}/projects/p/t.jsonl`,
    }),
  },
  {
    title: 'Window running, measurement missing',
    snapshot: inspectorSnapshot({
      sessionId: 'sess-5',
      transcriptPath: `${BLIND_ROOT}/projects/p/t.jsonl`,
    }),
  },
];

const inspectorSurface = (
  <main
    style={{
      padding: 16,
      fontFamily: 'var(--wks-font-sans)',
      background: 'var(--wks-bg-base)',
      color: 'var(--wks-text-primary)',
      minHeight: '100vh',
      boxSizing: 'border-box',
    }}
  >
    <h1 style={{ fontSize: '1.05rem' }}>Inspector · Usage</h1>
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12, alignItems: 'flex-start' }}>
      {inspectors.map((i) => (
        <section key={i.title} style={{ width: 320, maxWidth: '100%' }}>
          <h2 style={{ fontSize: '0.8rem', fontWeight: 600 }}>{i.title}</h2>
          <div
            style={{
              border: '1px solid var(--wks-border-subtle)',
              borderRadius: 'var(--wks-radius-md, 8px)',
              background: 'var(--wks-bg-surface)',
              maxHeight: 720,
              overflow: 'auto',
            }}
          >
            <InspectorCard snapshot={i.snapshot} initialTab="usage" />
          </div>
        </section>
      ))}
    </div>
  </main>
);

const overviewSurface = (
  <main
    style={{
      padding: 16,
      fontFamily: 'var(--wks-font-sans)',
      background: 'var(--wks-bg-base)',
      color: 'var(--wks-text-primary)',
      minHeight: '100vh',
      boxSizing: 'border-box',
    }}
  >
    <h1 style={{ fontSize: '1.05rem' }}>Overview usage</h1>
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 12 }}>
      {accounts.map((a, index) => (
        <UsageReportCard
          key={index}
          provider={index === 3 ? 'codex' : 'claude'}
          report={report}
          account={{
            account: a.account,
            label: a.label,
            source: 'disk',
            observed_at: now - 300,
            fresh: index === 3 ? false : null,
            windows: {
              five_hour: {
                used_percent: { state: 'ok', value: a.pct },
                resets_at: now + 9000,
                window_minutes: 300,
                pace: {
                  known: a.expected !== undefined,
                  state: 'ahead',
                  usedPct: a.pct ?? undefined,
                  expectedPct: a.expected,
                  curve: 'calendar',
                },
              },
            },
          }}
        />
      ))}
    </div>
    <h2 style={{ fontSize: '0.95rem', marginTop: 24 }}>Settings · Usage schedule</h2>
    <div style={{ maxWidth: 640 }}>
      <UsageScheduleRow />
    </div>
  </main>
);

createRoot(document.getElementById('root')!).render(
  params.get('surface') === 'inspector' ? inspectorSurface : overviewSurface,
);
