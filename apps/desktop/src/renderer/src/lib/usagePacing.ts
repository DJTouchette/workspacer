import {
  reportAccountKey,
  type UsageReportAccount,
  type UsageReportWire,
  type ReportWindowFields,
} from '../../../main/shared/usageReport';
import { claudeAccountOf } from './claudeAccount';
import { fmtResetAt, fmtResetIn } from './sessionStats';

const windows = [
  ['five_hour', '5h', 'fiveHour'],
  ['seven_day', '7d', 'sevenDay'],
  ['monthly', 'mo', 'monthly'],
] as const;
const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const percent = (v: unknown): v is number => finite(v) && v >= 0 && v <= 100;

/** The three comparisons the +/-2pp band can produce. `ahead` is the STATE's
 *  name (it mirrors the hub's own `PaceState`); what a user reads is
 *  `usagePaceLook`'s label, which is deliberately not that word. */
export type UsagePaceVerdict = 'ahead' | 'on pace' | 'under';

export interface UsagePacingRow {
  key: string;
  label: string;
  pct?: number;
  /** Used percentage as it is SHOWN, rounded once here so the word and the
   *  derived remainder below can never round to a pair summing to 101. */
  usedPct?: number;
  /** Allowance left, `100 - usedPct`. Derived ONLY from a measurement the
   *  report calls valid: an absent percentage leaves this absent too, because
   *  "no reading" rendered as "100% left" is the one failure a reader cannot
   *  detect. */
  left?: number;
  reset?: number;
  /** The reset, already spoken: `resets in 2h`, or a local wall clock when the
   *  window is more than a day out. */
  resetPhrase?: string;
  expected?: number;
  verdict?: UsagePaceVerdict;
  /** The reading is real history rather than a current observation, either
   *  because the provider aged it or because the report refresh failed. */
  stale: boolean;
  description: string;
}

/** How long a reset can be away and still read as a countdown. Past a day,
 *  `fmtResetIn`'s single coarsest unit ("3d") is too blunt to plan a weekly
 *  allowance around, so the row switches to a local wall clock. */
export const USAGE_RESET_ABSOLUTE_AFTER_MS = 24 * 60 * 60 * 1000;

/**
 * A window's reset, in words, in ONE place. The Overview card and the
 * Inspector's account block phrase it identically or they are two surfaces
 * describing one fact differently.
 *
 * A reset already in the past returns undefined rather than a phrase: the
 * caller has separately dropped that window as rolled over, and a leftover
 * "resets in 0m" would contradict it.
 */
export function usageResetPhrase(reset?: number, nowMs = Date.now()): string | undefined {
  if (!reset || reset * 1000 <= nowMs) return undefined;
  if (reset * 1000 - nowMs <= USAGE_RESET_ABSOLUTE_AFTER_MS) {
    // The SAME instant the window's currency was judged against, or the row
    // could survive the check and then count down from a different moment.
    const inWord = fmtResetIn(reset, nowMs);
    return inWord ? `resets in ${inWord}` : undefined;
  }
  const at = fmtResetAt(reset);
  return at ? `resets ${at}` : undefined;
}

/** The one-line explanation of the expected-pace tick. Colour and position
 *  alone do not say what the marker means, so every surface that draws a tick
 *  prints this beside it. */
export const USAGE_EXPECTED_LEGEND = 'Tick marks the share expected by now.';

/**
 * Why a reading is not current, when it is not.
 *
 * `usagePaceFallbackLabel` says the pace is unavailable; this says the
 * PERCENTAGE beside it is history. The two causes stay distinct because they
 * are different problems: the provider has not re-observed the account, or this
 * app could not re-read the report at all.
 */
export function usageStaleNote(
  report: UsageReportWire,
  account: UsageReportAccount,
): string | undefined {
  if (report.transport_stale)
    return 'Historical reading: the report refresh failed, so this is the last figure received.';
  if (account.fresh === false)
    return 'Historical reading: the provider last observed this account earlier.';
  return undefined;
}

/**
 * How one verdict is SHOWN. One mapper, so the word and the bar can never
 * disagree — they used to be two separate ternaries over `row.verdict`, and a
 * third surface would have been a third.
 *
 * Being over the curve is UNFAVOURABLE, and the presentation now says so:
 *
 *  - The word was `ahead`, which reads as praise. Ahead of schedule is good;
 *    ahead of your allowance is the opposite, and a user looking at a 7-day
 *    window at 89% with two days left should not see an encouraging word. It
 *    is `above pace`.
 *  - The colour was `--wks-warning`, the token this app uses for NEEDS-YOU
 *    (approval, input, stale). Overspending is not a prompt, it is a problem,
 *    so it takes `--wks-error` — the token DESIGN_LANGUAGE.md assigns to
 *    failure and danger.
 *
 * On pace and under stay NEUTRAL rather than turning green. A meter that
 * colours itself at every reading trains people to ignore the one reading that
 * is trying to say something, and `under` in particular is not a verdict worth
 * a colour: it is what most windows look like most of the time.
 *
 * NONE OF THIS IS A THRESHOLD CHANGE. The band that produces the verdict is
 * unchanged (inclusive +/-2 percentage points, compared before rounding), and
 * the hub's routing bands are not involved in this file at all.
 */
export function usagePaceLook(verdict: UsagePaceVerdict | undefined): {
  label: string | undefined;
  color: string;
} {
  if (verdict === 'ahead') return { label: 'above pace', color: 'var(--wks-error)' };
  return { label: verdict, color: 'var(--wks-text-secondary)' };
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
    // Rounded ONCE. The remainder is derived from the rounded figure rather
    // than rounded separately, so "72% used" is always paired with "28% left".
    const usedPct = pct === undefined ? undefined : Math.round(pct);
    const left = usedPct === undefined ? undefined : 100 - usedPct;
    const resetPhrase = usageResetPhrase(reset, nowMs);
    const stale = account.fresh === false || report.transport_stale === true;
    const description = [
      usedPct === undefined ? 'Usage unknown' : `${usedPct}% used`,
      left === undefined ? undefined : `${left}% of the allowance left`,
      resetPhrase,
      expected === undefined
        ? p?.state === 'disabled'
          ? 'Pacing disabled'
          : 'Pace unavailable'
        : `${Math.round(expected)}% expected by now · ${p?.curve} · evaluated ${new Date(report.evaluated_at! * 1000).toLocaleTimeString()}`,
      verdict === 'ahead' ? 'Above pace · spending faster than expected' : verdict,
      report.transport_stale ? 'Last known reading; report refresh failed' : undefined,
      account.fresh === false ? 'Stale provider observation' : undefined,
      account.source ? `Source: ${account.source}` : undefined,
      finite(account.observed_at)
        ? `Observed ${new Date(account.observed_at * 1000).toLocaleString()}`
        : 'Observation time unknown',
    ]
      .filter(Boolean)
      .join(' · ');
    rows.push({
      key,
      label,
      pct,
      usedPct,
      left,
      reset,
      resetPhrase,
      expected,
      verdict,
      stale,
      description,
    });
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

/**
 * The word shown where a verdict would be when the pace is not known.
 *
 * Shared so the Overview card and the Inspector's account block cannot describe
 * the same absence differently: a reading the provider itself has aged, or a
 * report whose refresh failed, is STALE — a distinct fact from a hub that
 * simply computes no pace for the window.
 */
export function usagePaceFallbackLabel(
  report: UsageReportWire,
  account: UsageReportAccount,
): string {
  return account.fresh === false || report.transport_stale
    ? 'Stale · pace unavailable'
    : 'Pace unavailable';
}

/**
 * Which report row — if any — describes the allowance THIS session spends
 * from.
 *
 * The Overview draws accounts with usable readings, so it never has to decide
 * whose card is whose. A session surface does, and the wrong answer is worse
 * than no answer: showing one login's 91% weekly figure on a session that runs
 * under a different login is a lie a reader cannot detect. So the only
 * attributions this makes are ones the session's own identity supports.
 *
 *  - `remote` — the session lives on a peer hub (`snapshot.hub`). This report
 *    is the LOCAL hub's, describing accounts on this machine; the peer's
 *    allowance is not in it and must not be guessed from it. Federation also
 *    blanks `transcriptPath`, which would otherwise collapse to the default
 *    account key and quietly borrow the local default login's numbers.
 *  - `match` — exactly one row answers to the session's identity. For Claude
 *    that is the config root behind its transcript path (`claudeAccountOf` vs
 *    `reportAccountKey`, the same vocabulary on both sides); for a provider
 *    with no per-session account marker it is the report having exactly ONE
 *    row for that provider, which is an identification rather than a guess.
 *  - `ambiguous` — the provider has several rows and nothing in the session
 *    picks one. Two Claude logins on one machine is the case this exists for.
 *  - `none` — the report was read and has nothing for this provider/account.
 *  - `unavailable` — there is no report at all (an older hub, a failed fetch,
 *    a backend with no `usageReport`). Callers render exactly what they
 *    rendered before the report existed.
 */
export type UsageReportAttribution =
  | { state: 'match'; provider: string; account: UsageReportAccount }
  | { state: 'ambiguous'; provider: string; count: number }
  | { state: 'none'; provider: string }
  | { state: 'remote'; provider: string; hub: string }
  | { state: 'unavailable'; provider: string };

export function usageReportAttribution(
  report: UsageReportWire | null | undefined,
  session: { provider?: string | null; transcriptPath?: string; hub?: string } | null | undefined,
): UsageReportAttribution {
  const provider = session?.provider || 'claude';
  if (!report) return { state: 'unavailable', provider };
  const hub = session?.hub;
  if (hub) return { state: 'remote', provider, hub };
  const rows = ((report.providers ?? []).find((p) => p?.provider === provider)?.accounts ?? [])
    .filter(Boolean)
    .map((a) => a as UsageReportAccount);
  if (!rows.length) return { state: 'none', provider };
  // Claude is the provider whose sessions carry their login in a path. Anything
  // else has no per-session marker, so `key` stays undefined and only a
  // single-row provider can be identified at all.
  const key =
    provider === 'claude' && session?.transcriptPath
      ? claudeAccountOf(session.transcriptPath)
      : undefined;
  if (key !== undefined) {
    // `reportAccountKey` is null for the report's UNATTRIBUTED bucket, which is
    // the daemon saying it could not name those sessions — never a match for a
    // session that can name itself.
    const matches = rows.filter((r) => reportAccountKey(r) === key);
    if (matches.length === 1) return { state: 'match', provider, account: matches[0] };
    if (!matches.length) return { state: 'none', provider };
    return { state: 'ambiguous', provider, count: matches.length };
  }
  if (rows.length === 1) return { state: 'match', provider, account: rows[0] };
  return { state: 'ambiguous', provider, count: rows.length };
}
