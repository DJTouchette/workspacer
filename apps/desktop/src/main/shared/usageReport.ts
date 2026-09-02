/**
 * claudemon's `GET /usage/report`, as both the main process and the renderer
 * read it.
 *
 * The report is the SESSION-FREE source of account rate-limit windows: it
 * answers for every provider and every configured account from what each CLI
 * left on disk (plus the daemon's own polled Claude reading), immediately, with
 * nothing running. Every other window source in the app rides on a live
 * session's status line and therefore says nothing at all on a cold start.
 *
 * These types live in `main/shared` rather than beside their first consumer
 * because there are now two: keep-warm (main) and the Overview usage card
 * (renderer), whose build graphs share only this directory. A second hand-typed
 * copy of a wire shape is how the usage plane already grew five mirrors of one
 * struct — see .rivet/context/domains/usage-accounting.md.
 *
 * THE CURRENCY RULE lives here too, and it is not a preference. A window's
 * `used_percent` is the last figure its provider wrote; for a window that has
 * rolled over, that figure is real history and a false present (live on
 * 2026-08-30 Codex reported 67% against a reset two days in the past). So a
 * reading is usable ONLY IF `resets_at` is present and strictly in the future
 * at the moment of the decision — the identical test
 * `services/hub/internal/limits/window.go ReadWindow` applies to the same
 * document, so the two cannot disagree.
 */

/** One scalar in the report. `ok` carries a reading; the other two carry a
 *  reason instead, and are NOT zero. */
export interface UsageReportMeasured {
  state: 'ok' | 'unknown' | 'unavailable';
  value?: number | null;
  reason?: string | null;
}

export interface UsageReportWindow {
  used_percent?: UsageReportMeasured | null;
  /** Epoch seconds. `null` when the source reported no reset time, which is
   *  also why the report leaves `is_current` null there. */
  resets_at?: number | null;
  window_minutes?: number | null;
  is_current?: boolean | null;
}

export interface UsageReportWindows {
  five_hour?: UsageReportWindow | null;
  seven_day?: UsageReportWindow | null;
  monthly?: UsageReportWindow | null;
}

export interface UsageReportAccount {
  /** `""` is the default login; `null` means the daemon cannot say which. */
  account?: string | null;
  label?: string | null;
  is_default?: boolean | null;
  windows?: UsageReportWindows | null;
}

export interface UsageReportProvider {
  provider: string;
  accounts?: UsageReportAccount[] | null;
  note?: string | null;
}

export interface UsageReportWire {
  generated_at?: number;
  providers?: UsageReportProvider[] | null;
}

/**
 * The window fields a usage surface renders, in the renderer's own vocabulary
 * (`sessionStats.UsageWindowSource` / a session status line). Structural on
 * purpose: the card feeds this straight into `usageWindows()` beside live
 * status-line fields, so the report cannot need its own rendering path.
 */
export interface ReportWindowFields {
  fiveHourPct?: number;
  fiveHourResetsAt?: number;
  fiveHourWindowMins?: number;
  sevenDayPct?: number;
  sevenDayResetsAt?: number;
  sevenDayWindowMins?: number;
  monthlyPct?: number;
  monthlyResetsAt?: number;
  monthlyWindowMins?: number;
}

/**
 * The account-group key a report row belongs to, in the renderer's vocabulary
 * (`lib/claudeAccount.claudeAccountOf`): `''` for the default login, the config
 * root's own directory name for a second one.
 *
 * `null` when the row names no account — the report's `unattributed` bucket,
 * which is a statement that the daemon could not attribute those sessions and
 * must never be folded into somebody's card.
 */
export function reportAccountKey(acct: UsageReportAccount): string | null {
  const raw = acct.account;
  if (raw == null) return null;
  const norm = raw.replace(/\\/g, '/').replace(/\/+$/, '');
  if (norm === '') return '';
  const base = norm.slice(norm.lastIndexOf('/') + 1);
  return base || (acct.label ?? '');
}

/**
 * One window, only if it is CURRENT. Returns null otherwise, which is what
 * keeps a rolled-over percentage and a reset time from a window that no longer
 * exists off the meter.
 *
 * `is_current` is honoured as a veto but not as a licence: the daemon computed
 * it at `generated_at` and this clock is the later one, so a window it called
 * current is still tested against `now` here.
 */
function currentWindow(
  w: UsageReportWindow | null | undefined,
  nowMs: number,
): { pct?: number; resetsAt: number; windowMins?: number } | null {
  if (!w) return null;
  if (w.is_current === false) return null;
  const resets = w.resets_at;
  if (resets == null) return null;
  if (resets * 1000 <= nowMs) return null;
  const m = w.used_percent;
  const pct = m && m.state === 'ok' && typeof m.value === 'number' ? m.value : undefined;
  return {
    pct,
    resetsAt: resets,
    windowMins: typeof w.window_minutes === 'number' ? w.window_minutes : undefined,
  };
}

/**
 * The current windows the report holds for one provider and account group, in
 * the shape a usage surface renders.
 *
 * `account` undefined means "this provider has one account here" (every
 * non-Claude card): the default row wins, else the first row that has anything
 * current to say. An empty object is a truthful answer — the report was read
 * and no window of that account is running — and is what stops the card
 * drawing a 0% meter over a window nobody reported.
 */
export function reportWindowsFor(
  report: UsageReportWire | null | undefined,
  provider: string,
  account: string | undefined,
  nowMs: number,
): ReportWindowFields {
  const rows = (report?.providers ?? []).find((p) => p?.provider === provider)?.accounts ?? [];
  const candidates =
    account === undefined
      ? [...rows].sort((a, b) => Number(!!b?.is_default) - Number(!!a?.is_default))
      : rows.filter((a) => a && reportAccountKey(a) === account);

  for (const row of candidates) {
    if (!row) continue;
    const five = currentWindow(row.windows?.five_hour, nowMs);
    const seven = currentWindow(row.windows?.seven_day, nowMs);
    const monthly = currentWindow(row.windows?.monthly, nowMs);
    if (!five && !seven && !monthly) continue;
    const out: ReportWindowFields = {};
    if (five) {
      if (five.pct !== undefined) out.fiveHourPct = five.pct;
      out.fiveHourResetsAt = five.resetsAt;
      if (five.windowMins !== undefined) out.fiveHourWindowMins = five.windowMins;
    }
    if (seven) {
      if (seven.pct !== undefined) out.sevenDayPct = seven.pct;
      out.sevenDayResetsAt = seven.resetsAt;
      if (seven.windowMins !== undefined) out.sevenDayWindowMins = seven.windowMins;
    }
    if (monthly) {
      if (monthly.pct !== undefined) out.monthlyPct = monthly.pct;
      out.monthlyResetsAt = monthly.resetsAt;
      if (monthly.windowMins !== undefined) out.monthlyWindowMins = monthly.windowMins;
    }
    return out;
  }
  return {};
}

/** The Claude account groups the report has a CURRENT window for. Lets the
 *  Overview draw a second login's card on a cold start, where there is no
 *  session and no remembered reading to discover it from. */
export function reportAccountKeys(
  report: UsageReportWire | null | undefined,
  provider: string,
  nowMs: number,
): string[] {
  const rows = (report?.providers ?? []).find((p) => p?.provider === provider)?.accounts ?? [];
  const keys: string[] = [];
  for (const row of rows) {
    if (!row) continue;
    const key = reportAccountKey(row);
    if (key === null || keys.includes(key)) continue;
    const w = row.windows;
    if (!currentWindow(w?.five_hour, nowMs) && !currentWindow(w?.seven_day, nowMs)) continue;
    keys.push(key);
  }
  return keys;
}
