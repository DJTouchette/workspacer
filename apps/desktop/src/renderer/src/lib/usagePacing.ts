import type {
  UsageReportAccount,
  UsageReportWire,
  ReportWindowFields,
} from '../../../main/shared/usageReport';

const windows = [
  ['five_hour', '5h', 'fiveHour'],
  ['seven_day', '7d', 'sevenDay'],
  ['monthly', 'mo', 'monthly'],
] as const;
const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const percent = (v: unknown): v is number => finite(v) && v >= 0 && v <= 100;

export interface UsagePacingRow {
  key: string;
  label: string;
  pct?: number;
  reset?: number;
  expected?: number;
  verdict?: 'ahead' | 'on pace' | 'under';
  description: string;
}

/** One report account, never overlaid with session fields or another observation.
 * The +/-2pp presentation band is inclusive and compared BEFORE rounding. It
 * does not change the hub's routing state or thresholds.
 */
export function usagePacingRows(
  report: UsageReportWire,
  account: UsageReportAccount,
  nowMs: number,
) {
  const fields: ReportWindowFields = {};
  const rows: UsagePacingRow[] = [];
  for (const [key, label, prefix] of windows) {
    const w = account.windows?.[key];
    if (!w || w.used_percent?.state === 'unavailable') continue;
    const reset = finite(w.resets_at) && w.resets_at > 0 ? w.resets_at : undefined;
    if (w.is_current === false || (reset !== undefined && reset * 1000 <= nowMs)) continue;
    const current = reset !== undefined;
    const pct =
      current && w.used_percent?.state === 'ok' && percent(w.used_percent.value)
        ? w.used_percent.value
        : undefined;
    const length =
      finite(w.window_minutes) && w.window_minutes > 0 && w.window_minutes <= 366 * 24 * 60
        ? w.window_minutes
        : undefined;
    const p = w.pace;
    const sampleCurrent =
      !report.transport_stale &&
      finite(report.evaluated_at) &&
      report.evaluated_at * 1000 <= nowMs &&
      finite(report.valid_until) &&
      nowMs < report.valid_until * 1000 &&
      nowMs < (report.evaluated_at + 60) * 1000;
    const expected =
      current &&
      length !== undefined &&
      reset * 1000 - nowMs <= length * 60000 &&
      sampleCurrent &&
      account.fresh !== false &&
      p?.known === true &&
      ['on_track', 'ahead', 'overspending'].includes(p.state) &&
      typeof p.curve === 'string' &&
      p.curve.length > 0 &&
      percent(p.expectedPct) &&
      percent(p.usedPct) &&
      p.usedPct === pct
        ? p.expectedPct
        : undefined;
    const delta = expected !== undefined && pct !== undefined ? pct - expected : undefined;
    const verdict =
      delta === undefined ? undefined : delta > 2 ? 'ahead' : delta < -2 ? 'under' : 'on pace';
    const description = [
      pct === undefined ? 'Usage unknown' : `${Math.round(pct)}% used`,
      expected === undefined
        ? p?.state === 'disabled'
          ? 'Pacing disabled'
          : 'Pace unavailable'
        : `${Math.round(expected)}% expected by now · ${p?.curve} · evaluated ${new Date(report.evaluated_at! * 1000).toLocaleTimeString()}`,
      verdict === 'ahead' ? 'Spending faster than expected' : verdict,
      report.transport_stale ? 'Last known reading; report refresh failed' : undefined,
      account.fresh === false ? 'Stale provider observation' : undefined,
      account.source ? `Source: ${account.source}` : undefined,
      finite(account.observed_at)
        ? `Observed ${new Date(account.observed_at * 1000).toLocaleString()}`
        : 'Observation time unknown',
    ]
      .filter(Boolean)
      .join(' · ');
    rows.push({ key, label, pct, reset, expected, verdict, description });
    if (pct !== undefined) fields[`${prefix}Pct`] = pct;
    if (reset !== undefined) fields[`${prefix}ResetsAt`] = reset;
    if (length !== undefined && current) fields[`${prefix}WindowMins`] = length;
  }
  return { rows, fields };
}

export function usageAccountIdentity(account: UsageReportAccount): string {
  return account.account == null
    ? 'Unattributed account'
    : account.account === ''
      ? 'Default account'
      : account.account;
}
