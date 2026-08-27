package main

// `brief.check` — the read-only half of brief maintenance, headless: which
// `## Now` lines are talking about workers that no longer exist.
//
// TWIN: apps/desktop/src/main/services/briefCheck.ts, held equal by
// contracts/brief-board-cases.json. That file carries the full argument; the
// short version is that a Now line does not remove itself when its worker dies,
// because the wake that would remind the manager to move it is the same wake
// handing it a result to act on, and the line loses. The cost lands on the NEXT
// manager, which reads four dispatch lines, believes four workers are running,
// and goes looking for sessions that ended days ago.
//
// IT FLAGS. IT NEVER TOUCHES THE FILE. Doctrine, not a limitation: the brief is
// the USER'S document, their own edits are authoritative, and every write path
// in this package is additive (brief.go) or move-only (briefboard.go) for
// exactly that reason. A checker that pruned would be the one component able to
// destroy hand-written prose on the strength of a heuristic, and the heuristic
// is wrong sometimes — a Now line may name a session that ended BECAUSE the work
// is genuinely still in flight under a successor. So it returns a report and the
// manager decides.
//
// PRECISION OVER RECALL, deliberately, exactly as the twin:
//   - a line whose session reference resolves to a KNOWN session is silent;
//   - a line with NO reference is flagged only when it is unmistakably
//     dispatch-shaped (it says "dispatched"/"dispatching"), never on a hunch;
//   - anything else is left alone.
//
// A FINISHED SESSION COUNTS AS GONE. The question a Now line answers is "is this
// dispatch still running", not "did this session ever exist". A worker that
// finished cleanly and was closed is precisely the case that leaves the line
// behind, so treating it as live would blind the check to its main quarry.
//
// AND WHERE THIS SIDE CANNOT ANSWER THAT, IT SAYS SO. The liveness source here
// is the brain's own live session store, which exists in FULL scope only. With
// no store there is no way to tell a finished dispatch from a running one, and
// the wrong move would be to run the comparison anyway: every reference would
// resolve to nothing and the report would flag the whole section as stale — a
// checker that cries wolf is one a manager stops reading. So the stale arm is
// SKIPPED and named in `unavailableChecks`, and the two arms that need no
// session store (a malformed reference, a dispatch line with no reference at
// all) still run. See briefCheckCall.

import (
	"context"
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
)

// briefCheckedSection is the section this checks. `Now` is the only section
// with an expiry: Direction is durable, Recently is a dated log that is SUPPOSED
// to name dead sessions, and User is the user's own standing preferences.
// TWIN: CHECKED_SECTION.
const briefCheckedSection = "Now"

var (
	// Any `session:<token>` at all, INCLUDING malformed ones — the point is to
	// catch `session:6a-round2`, so this is deliberately looser than the strict
	// form normalizeSessionRef enforces. TWIN: ANY_SESSION_TOKEN_RE.
	anyBriefSessionTokenRe = regexp.MustCompile(`session:([A-Za-z0-9_.-]+)`)
	// The only no-reference shape confident enough to flag. Narrow on purpose:
	// see the header on precision. TWIN: DISPATCH_SHAPED_RE.
	briefDispatchShapedRe = regexp.MustCompile(`(?i)\bdispatch(?:ed|ing)?\b`)
)

// briefNowFinding is one flagged entry. TWIN: BriefNowFinding.
type briefNowFinding struct {
	// Line is a 0-based index into the brief's lines, so a manager can find the
	// entry.
	Line int `json:"line"`
	// Text is the entry's first line, as written. Never rewritten.
	Text string `json:"text"`
	// Reason is one of stale | malformed | unreferenced.
	Reason string `json:"reason"`
	// Refs are the references this finding is about, canonicalized where
	// possible. Never null on the wire: an absent list and an empty one read
	// the same to a model, and the twin emits [].
	Refs []string `json:"refs"`
	// Detail is one sentence naming what to do about it.
	Detail string `json:"detail"`
}

// briefCheckReport is what the capability answers with. TWIN: BriefCheckReport,
// plus UnavailableChecks — see this file's header.
type briefCheckReport struct {
	Path    string `json:"path"`
	Section string `json:"section"`
	// EntriesChecked is how many `## Now` entries were examined.
	EntriesChecked int `json:"entriesChecked"`
	// EntriesLive is how many resolve to a session this host still knows about.
	EntriesLive int               `json:"entriesLive"`
	Findings    []briefNowFinding `json:"findings"`
	// LiveSessions is how many live sessions the check was matched against.
	// Zero is worth seeing: it means EVERY reference will look stale, which is
	// a fleet that is idle (or a store that has not hydrated), not a brief that
	// is rotten.
	LiveSessions int `json:"liveSessions"`
	// UnavailableChecks names the arms this provider could NOT run, so a
	// narrowed report is never mistaken for a clean one. Absent (omitted, not
	// empty) when every arm ran, which is the desktop twin's shape exactly.
	UnavailableChecks []string `json:"unavailableChecks,omitempty"`
	Note              string   `json:"note"`
}

// isLiveDispatch reports whether a session row is one a Now line could still
// legitimately be about.
//
// Deliberately NOT the fleet-visibility rule (visibility.go) and not a security
// predicate: those answer "should a client SEE this row", which refuses a
// federated row and a bare terminal for reasons that have nothing to do with
// this one. A dispatch on a peer hub is a live dispatch, and refusing it here
// would flag a perfectly current Now line.
//
// The clauses that DO carry over are the death ones, in both spellings the
// store uses — claudemon's `mode` and the desktop-shaped `status` the compat
// overlay writes — plus archived. A row that will not decode its mode or status
// is treated as LIVE, which is right for the same reason the security
// predicate's opposite default is: there the failure to avoid is widening a
// grant, here it is flagging a line that is fine. TWIN: isLiveDispatch.
func isLiveDispatch(row map[string]any) bool {
	if row == nil {
		return false
	}
	if archived, ok := row["archived"].(bool); ok && archived {
		return false
	}
	mode, _ := row["mode"].(string)
	status, _ := row["status"].(string)
	if mode == "stopped" || mode == "ended" {
		return false
	}
	if status == "ended" || status == "stopped" {
		return false
	}
	return true
}

// liveSessionIDs are the ids of the rows a Now line may still be about,
// lowercased. TWIN: liveSessionIds.
//
// The one addition on this side is the `session_id` fallback: the desktop store
// holds camelCase snapshots only, while this brain's store holds claudemon rows
// that USUALLY carry the compat overlay's `sessionId` and — for a row that
// never went through enrichAndCompat — carry only the snake_case original. The
// same fallback fleetSessions makes, for the same reason: a row read as
// id-less is an invisible session, which is a wrong answer rather than a
// missing one, and here it would flag a live worker's Now line as stale.
func liveSessionIDs(rows []map[string]any) []string {
	out := []string{}
	for _, row := range rows {
		if !isLiveDispatch(row) {
			continue
		}
		id, _ := row["sessionId"].(string)
		if strings.TrimSpace(id) == "" {
			id, _ = row["session_id"].(string)
		}
		if strings.TrimSpace(id) == "" {
			continue
		}
		out = append(out, strings.ToLower(strings.TrimSpace(id)))
	}
	return out
}

// briefRefResolves matches a brief's SHORT reference against the store's full
// UUIDs by prefix in whichever direction is longer. Both sides are already
// known to be hex runs of 6+, which is long enough that a prefix collision
// between two real sessions is not a practical concern. TWIN: resolves.
func briefRefResolves(ref string, live []string) bool {
	for _, id := range live {
		if strings.HasPrefix(id, ref) || strings.HasPrefix(ref, id) {
			return true
		}
	}
	return false
}

// checkNowSection reads `## Now` and reports which entries are talking about
// sessions that are gone. PURE — it is handed the brief's text and the live
// ids, and returns a report. Nothing here can write, which is the guarantee this
// file's header makes and a test pins directly. TWIN: checkNowSection.
func checkNowSection(content string, live []string, briefPath string) briefCheckReport {
	liveIDs := make([]string, 0, len(live))
	for _, s := range live {
		if t := strings.ToLower(strings.TrimSpace(s)); t != "" {
			liveIDs = append(liveIDs, t)
		}
	}
	doc := parseBriefDoc(content)
	var entries []briefEntry
	for _, e := range doc.Entries {
		if strings.EqualFold(strings.TrimSpace(e.Column), briefCheckedSection) {
			entries = append(entries, e)
		}
	}

	findings := []briefNowFinding{}
	entriesLive := 0

	for _, entry := range entries {
		var good, bad []string
		for _, m := range anyBriefSessionTokenRe.FindAllStringSubmatch(entry.Text, -1) {
			token := m[1]
			if ref, err := normalizeSessionRef(token); err == nil {
				if !containsRef(good, ref) {
					good = append(good, ref)
				}
			} else if !containsRef(bad, token) {
				bad = append(bad, token)
			}
		}

		if len(bad) > 0 {
			// The transcription bug, caught where it landed. Reported even when
			// the entry ALSO carries a good reference: half a broken line is
			// still a broken link in the user's brief.
			findings = append(findings, briefNowFinding{
				Line: entry.Start, Text: entry.Lines[0], Reason: "malformed", Refs: bad,
				Detail: fmt.Sprintf("%s is not a session id, so it links to nothing. Fix the reference by hand "+
					"(or re-append the line with brief_append's sessionId param, which validates it) — "+
					"this check does not edit the brief.", joinSessionRefs(bad)),
			})
			if anyRefResolves(good, liveIDs) {
				entriesLive++
			}
			continue
		}

		if len(good) > 0 {
			if anyRefResolves(good, liveIDs) {
				entriesLive++
				continue
			}
			findings = append(findings, briefNowFinding{
				Line: entry.Start, Text: entry.Lines[0], Reason: "stale", Refs: good,
				Detail: fmt.Sprintf("%s is not a session this host still knows about, so this Now line has "+
					"outlived its dispatch. If the work landed, move the entry to Recently (or archive it); "+
					"if it is still yours, re-dispatch and write the new session id. Nothing was changed.",
					joinSessionRefs(good)),
			})
			continue
		}

		// No reference at all. Flagged only when the wording leaves no doubt —
		// see the header on precision.
		if briefDispatchShapedRe.MatchString(entry.Text) {
			findings = append(findings, briefNowFinding{
				Line: entry.Start, Text: entry.Lines[0], Reason: "unreferenced", Refs: []string{},
				Detail: "this reads like a dispatch but names no session:<id>, so nothing can tell you " +
					"whether its worker is still alive. Add the reference when you next touch the line.",
			})
		}
	}

	return briefCheckReport{
		Path:           briefPath,
		Section:        briefCheckedSection,
		EntriesChecked: len(entries),
		EntriesLive:    entriesLive,
		Findings:       findings,
		LiveSessions:   len(liveIDs),
		Note:           briefCheckNote(len(entries), findings),
	}
}

// briefCheckNote is the report's closing sentence, which is the part a manager
// actually acts on. TWIN: the `note` ternary in checkNowSection.
func briefCheckNote(entries int, findings []briefNowFinding) string {
	if len(findings) == 0 {
		return fmt.Sprintf("Every ## Now entry (%d) either names a session this host still knows about "+
			"or is not a dispatch line. Nothing to prune.", entries)
	}
	stale := 0
	for _, f := range findings {
		if f.Reason == "stale" {
			stale++
		}
	}
	staleClause := ""
	if stale > 0 {
		staleClause = fmt.Sprintf(" (%d name sessions that are gone)", stale)
	}
	return fmt.Sprintf("%d of %d ## Now entries need YOUR judgement%s. This check only reports: it never "+
		"edits, moves or deletes a line, because the user's own brief edits are authoritative. "+
		"Act on them with a board move or an explicit edit.", len(findings), entries, staleClause)
}

func joinSessionRefs(refs []string) string {
	out := make([]string, 0, len(refs))
	for _, r := range refs {
		out = append(out, "session:"+r)
	}
	return strings.Join(out, ", ")
}

func anyRefResolves(refs, live []string) bool {
	for _, r := range refs {
		if briefRefResolves(r, live) {
			return true
		}
	}
	return false
}

func containsRef(list []string, want string) bool {
	for _, s := range list {
		if s == want {
			return true
		}
	}
	return false
}

// withoutLivenessChecks degrades a report that was computed with NO liveness
// source, rather than letting it lie.
//
// It drops the `stale` findings — with an empty live set every reference
// resolves to nothing, so every referenced entry would be flagged and the
// report would say the whole section is rotten — and it zeroes `entriesLive`,
// which is unknowable rather than zero. What survives are the two arms that
// never needed a session store: a malformed reference links to nothing whatever
// is running, and a dispatch line with no reference at all is unanswerable by
// construction.
func withoutLivenessChecks(rep briefCheckReport, why string) briefCheckReport {
	kept := []briefNowFinding{}
	for _, f := range rep.Findings {
		if f.Reason != "stale" {
			kept = append(kept, f)
		}
	}
	rep.Findings = kept
	rep.EntriesLive = 0
	rep.LiveSessions = 0
	rep.UnavailableChecks = []string{"stale"}
	rep.Note = briefCheckNote(rep.EntriesChecked, kept) +
		" ONE CHECK DID NOT RUN: " + why + " — so no entry here was tested for a dead dispatch, " +
		"and this report is not evidence that there are none. The malformed-reference and " +
		"no-reference checks did run."
	return rep
}

// briefCheckCall is the capability. It READS and returns; nothing in this path
// can write.
//
// Confinement is brief.append's, unchanged and for the same reason: `project`
// is the caller's only path input, it is held to the workspace roots fs.write
// takes, and the COMPOSED path is asserted too — `project` can be a legitimate
// allowed directory while `<project>/.workspacer` is a symlink pointing out of
// every root, and a guard that only ever resolved `project` answers yes,
// truthfully, about a path that is not the one being opened.
func (r *registry) briefCheckCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Project string `json:"project"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if strings.TrimSpace(p.Project) == "" {
		return nil, fmt.Errorf("brief.check requires { project }")
	}
	roots := r.workspaceRoots(ctx)
	dir, err := assertPathAllowed("brief.check", p.Project, roots)
	if err != nil {
		return nil, err
	}
	briefPath, err := assertPathAllowed("brief.check", briefPathFor(dir), roots)
	if err != nil {
		return nil, err
	}
	// A project with no brief is not an error — it has no stale Now lines,
	// which is the honest answer and the one a manager can act on.
	content, _, err := readBriefOrEmpty(briefPath)
	if err != nil {
		return nil, err
	}

	if r.store == nil {
		return jsonResult(withoutLivenessChecks(
			checkNowSection(content, nil, briefPath),
			"this brain holds no live session store (it is registered in catalog scope), so it cannot tell "+
				"a finished dispatch from a running one"))
	}
	return jsonResult(checkNowSection(content, liveSessionIDs(r.storeRows()), briefPath))
}

// storeRows decodes the live session store into plain maps for the liveness
// read. Rows that do not decode are dropped rather than guessed at — the twin
// asks the same questions of an `unknown` and answers false for a non-object.
func (r *registry) storeRows() []map[string]any {
	if r.store == nil {
		return nil
	}
	snaps := r.store.all()
	out := make([]map[string]any, 0, len(snaps))
	for _, snap := range snaps {
		var row map[string]any
		if json.Unmarshal(snap, &row) != nil || row == nil {
			continue
		}
		out = append(out, row)
	}
	return out
}
