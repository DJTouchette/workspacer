/**
 * `brief_check` — the read-only half of brief maintenance: which `## Now` lines
 * are talking about workers that no longer exist.
 *
 * WHY. Every brief in this fleet has recorded the same lesson in its own words:
 * *a Now line does not remove itself when its worker dies.* `Now` is the
 * in-flight section, so a manager writes a line when it dispatches and is
 * supposed to move it when the worker lands. It reliably does not — the wake
 * that would remind it is the same wake that hands it the result to act on, and
 * the line loses. The visible cost is a successor manager reading four dispatch
 * lines and believing four workers are running when none are, then "checking
 * on" sessions that ended days ago.
 *
 * IT FLAGS. IT NEVER TOUCHES THE FILE. This is doctrine here and not a
 * preference: a brief is the USER'S document, their own edits are authoritative,
 * and every write path in this codebase (briefService, briefBoardService) is
 * additive or move-only for exactly that reason. A checker that deleted a "stale"
 * line would be the one component able to destroy hand-written prose, on the
 * strength of a heuristic — and the heuristic is wrong sometimes: a Now line may
 * name a session that ended BECAUSE the work is genuinely still in flight under a
 * successor, or may be a standing note the user wrote themselves. So the output
 * is a REPORT a manager reads and acts on with its own judgement, through the
 * same board move or brief edit it would have used anyway.
 *
 * PRECISION OVER RECALL, deliberately. A checker that cries wolf is one a
 * manager stops reading, and an ignored checker is worse than none because it
 * still costs a tool call. So:
 *   - a line whose session reference resolves to a KNOWN session is silent;
 *   - a line with NO reference is flagged only when it is unmistakably
 *     dispatch-shaped (it says "dispatched"/"dispatching"), never on a hunch;
 *   - anything else is left alone.
 *
 * A FINISHED SESSION COUNTS AS GONE. The question a Now line answers is "is
 * this dispatch still running", not "did this session ever exist". A worker that
 * finished cleanly and was closed is precisely the case that leaves the line
 * behind, so treating it as live would blind the check to its main quarry.
 */
import { parseBrief } from '../shared/briefBoard';
import { isSessionRef, normalizeSessionRef } from '../lib/briefResultLine';

/** The section this checks. `Now` is the only section with an expiry: Direction
 *  is durable, Recently is a dated log that is SUPPOSED to name dead sessions,
 *  and User is the user's own standing preferences. */
export const CHECKED_SECTION = 'Now';

/** Any `session:<token>` token at all, INCLUDING malformed ones — the point is
 *  to catch `session:6a-round2`, so this must be looser than the strict form in
 *  briefResultLine and briefBoard. */
const ANY_SESSION_TOKEN_RE = /session:([A-Za-z0-9_.-]+)/g;

/** The only no-reference shape confident enough to flag. Narrow on purpose: see
 *  the module header on precision. */
const DISPATCH_SHAPED_RE = /\bdispatch(?:ed|ing)?\b/i;

export type BriefFindingReason = 'stale' | 'malformed' | 'unreferenced';

export interface BriefNowFinding {
  /** 0-based index into the brief's lines, so a manager can find the entry. */
  line: number;
  /** The entry's first line, as written. Never rewritten. */
  text: string;
  reason: BriefFindingReason;
  /** The references this finding is about, canonicalized where possible. */
  refs: string[];
  /** One sentence naming what to do about it. */
  detail: string;
}

export interface BriefCheckReport {
  path: string;
  section: typeof CHECKED_SECTION;
  /** Entries under `## Now` that were examined. */
  entriesChecked: number;
  /** Entries whose reference resolves to a known session — the healthy ones. */
  entriesLive: number;
  findings: BriefNowFinding[];
  /** How many live sessions the check was matched against. Zero is worth
   *  seeing: it means EVERY reference will look stale, which is a fleet that is
   *  idle (or a store that has not hydrated), not a brief that is rotten. */
  liveSessions: number;
  note: string;
}

/**
 * Is this session row one a Now line could still legitimately be about?
 *
 * Deliberately NOT `snapshotGrantsFsRoot`: that predicate answers a SECURITY
 * question ("may this row hand out a filesystem root") and refuses a federated
 * row and a bare terminal for reasons that have nothing to do with this one. A
 * dispatch on a peer hub is a live dispatch, and refusing it here would flag a
 * perfectly current Now line.
 *
 * The clauses that DO carry over are the death ones, in both spellings the
 * store uses (claudemon's `mode`, the desktop-shaped `status`), plus archived.
 * A row that will not decode is treated as LIVE, which is the opposite of the
 * security predicate's default and right for the same underlying reason: there,
 * the failure mode to avoid is widening a grant; here, it is flagging a line
 * that is fine.
 */
export function isLiveDispatch(snap: unknown): boolean {
  if (typeof snap !== 'object' || snap === null || Array.isArray(snap)) return false;
  const row = snap as Record<string, unknown>;
  if (row.archived === true) return false;
  const mode = typeof row.mode === 'string' ? row.mode : '';
  const status = typeof row.status === 'string' ? row.status : '';
  if (mode === 'stopped' || mode === 'ended') return false;
  if (status === 'ended' || status === 'stopped') return false;
  return true;
}

/** Session ids of the rows a Now line may still be about, lowercased. */
export function liveSessionIds(snapshots: unknown[]): string[] {
  const out: string[] = [];
  for (const snap of snapshots) {
    if (!isLiveDispatch(snap)) continue;
    const id = (snap as Record<string, unknown>).sessionId;
    if (typeof id === 'string' && id.trim() !== '') out.push(id.trim().toLowerCase());
  }
  return out;
}

/** A brief writes SHORT references and the store holds full UUIDs, so matching
 *  is by prefix in whichever direction is longer. Both sides are already known
 *  to be hex runs of 6+, which is long enough that a prefix collision between
 *  two real sessions is not a practical concern. */
function resolves(ref: string, live: string[]): boolean {
  return live.some((id) => id.startsWith(ref) || ref.startsWith(id));
}

/**
 * Read `## Now` and report which entries are talking about sessions that are
 * gone. PURE — it is handed the brief's text and the live ids, and returns a
 * report. Nothing here can write, which is the guarantee the module header
 * makes and a test pins directly.
 */
export function checkNowSection(
  content: string,
  live: string[],
  briefPath: string,
): BriefCheckReport {
  const liveIds = live.map((s) => s.trim().toLowerCase()).filter((s) => s !== '');
  const doc = parseBrief(content);
  const entries = doc.entries.filter(
    (e) => e.column.trim().toLowerCase() === CHECKED_SECTION.toLowerCase(),
  );

  const findings: BriefNowFinding[] = [];
  let entriesLive = 0;

  for (const entry of entries) {
    const good: string[] = [];
    const bad: string[] = [];
    ANY_SESSION_TOKEN_RE.lastIndex = 0;
    for (const m of entry.text.matchAll(ANY_SESSION_TOKEN_RE)) {
      const token = m[1];
      if (isSessionRef(token)) {
        const ref = normalizeSessionRef(token);
        if (!good.includes(ref)) good.push(ref);
      } else if (!bad.includes(token)) {
        bad.push(token);
      }
    }

    if (bad.length > 0) {
      // The transcription bug, caught where it landed. Reported even when the
      // entry ALSO carries a good reference: half a broken line is still a
      // broken link in the user's brief.
      findings.push({
        line: entry.start,
        text: entry.lines[0],
        reason: 'malformed',
        refs: bad,
        detail:
          `${bad.map((b) => `session:${b}`).join(', ')} is not a session id, so it links to ` +
          "nothing. Fix the reference by hand (or re-append the line with brief_append's " +
          'sessionId param, which validates it) — this check does not edit the brief.',
      });
      if (good.some((ref) => resolves(ref, liveIds))) entriesLive++;
      continue;
    }

    if (good.length > 0) {
      if (good.some((ref) => resolves(ref, liveIds))) {
        entriesLive++;
        continue;
      }
      findings.push({
        line: entry.start,
        text: entry.lines[0],
        reason: 'stale',
        refs: good,
        detail:
          `${good.map((r) => `session:${r}`).join(', ')} is not a session this host still ` +
          'knows about, so this Now line has outlived its dispatch. If the work landed, move ' +
          'the entry to Recently (or archive it); if it is still yours, re-dispatch and write ' +
          'the new session id. Nothing was changed.',
      });
      continue;
    }

    // No reference at all. Flagged only when the wording leaves no doubt — see
    // the module header on precision.
    if (DISPATCH_SHAPED_RE.test(entry.text)) {
      findings.push({
        line: entry.start,
        text: entry.lines[0],
        reason: 'unreferenced',
        refs: [],
        detail:
          'this reads like a dispatch but names no session:<id>, so nothing can tell you ' +
          'whether its worker is still alive. Add the reference when you next touch the line.',
      });
    }
  }

  const stale = findings.filter((f) => f.reason === 'stale').length;
  return {
    path: briefPath,
    section: CHECKED_SECTION,
    entriesChecked: entries.length,
    entriesLive,
    findings,
    liveSessions: liveIds.length,
    note:
      findings.length === 0
        ? `Every ## Now entry (${entries.length}) either names a session this host still knows ` +
          'about or is not a dispatch line. Nothing to prune.'
        : `${findings.length} of ${entries.length} ## Now entries need YOUR judgement` +
          (stale ? ` (${stale} name sessions that are gone)` : '') +
          '. This check only reports: it never edits, moves or deletes a line, because the ' +
          "user's own brief edits are authoritative. Act on them with a board move or an " +
          'explicit edit.',
  };
}
