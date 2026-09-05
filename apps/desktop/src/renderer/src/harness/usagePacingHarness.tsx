/** Isolated visual fixture: real account cards, no daemon, credentials or probing. */
import React from 'react';
import { createRoot } from 'react-dom/client';
import '../App.css';
import { applyTheme, darkTheme, lightTheme } from '../themes';
import { UsageReportCard } from '../components/UsageReportCard';
import UsageScheduleRow from '../components/settings/UsageScheduleRow';
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
createRoot(document.getElementById('root')!).render(
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
                  usedPct: a.pct,
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
  </main>,
);
