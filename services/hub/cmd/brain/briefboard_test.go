package main

// The Go half of contracts/brief-board-cases.json, plus the behaviour that is
// this side's alone.
//
// The TS half is apps/desktop/src/main/services/briefBoardContract.test.ts. Both
// providers answer brief.archive and brief.check now, so a Fleet Manager must
// not get a different answer about its own brief depending on which one ran —
// and the thing most likely to drift is not the verb but the ENTRY BOUNDARY
// underneath it. That is what the fixture pins, case by case.

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

const briefBoardFixtureRel = "contracts/brief-board-cases.json"

// briefBoardOwnerKeys are this side's keys in the fixture's `owners` map.
// Dropping either file out of `owners` would otherwise run every case and prove
// nothing about whether THESE copies are still on the hook.
var briefBoardOwnerKeys = []string{
	"services/hub/cmd/brain/briefboard.go",
	"services/hub/cmd/brain/briefcheck.go",
}

type briefEntryExpect struct {
	Column string   `json:"column"`
	Group  string   `json:"group"`
	Start  int      `json:"start"`
	End    int      `json:"end"`
	Lines  []string `json:"lines"`
}

type briefFindingExpect struct {
	Line   int      `json:"line"`
	Text   string   `json:"text"`
	Reason string   `json:"reason"`
	Refs   []string `json:"refs"`
}

type briefBoardFixture struct {
	Owners  map[string]string `json:"owners"`
	Entries []struct {
		Name   string             `json:"name"`
		Brief  string             `json:"brief"`
		Expect []briefEntryExpect `json:"expect"`
		Why    string             `json:"why"`
	} `json:"entries"`
	Archive []struct {
		Name    string   `json:"name"`
		Brief   string   `json:"brief"`
		Archive string   `json:"archive"`
		Section string   `json:"section"`
		Count   *float64 `json:"count"`
		Keep    *float64 `json:"keep"`
		Date    string   `json:"date"`
		Expect  struct {
			Brief            string `json:"brief"`
			Archive          string `json:"archive"`
			Archived         int    `json:"archived"`
			EntriesInSection int    `json:"entriesInSection"`
			BytesInSection   int    `json:"bytesInSection"`
			BytesInBrief     int    `json:"bytesInBrief"`
		} `json:"expect"`
		Why string `json:"why"`
	} `json:"archive"`
	Check []struct {
		Name     string           `json:"name"`
		Brief    string           `json:"brief"`
		Sessions []map[string]any `json:"sessions"`
		Expect   struct {
			EntriesChecked int                  `json:"entriesChecked"`
			EntriesLive    int                  `json:"entriesLive"`
			LiveSessions   int                  `json:"liveSessions"`
			Findings       []briefFindingExpect `json:"findings"`
		} `json:"expect"`
		Why string `json:"why"`
	} `json:"check"`
}

// The per-block floors. "not zero" is met by a corpus that lost every case but
// one, which is the shape this repo keeps re-finding.
const (
	briefEntryCorpusFloor   = 4
	briefArchiveCorpusFloor = 5
	briefCheckCorpusFloor   = 6
)

func loadBriefBoardFixture(t *testing.T) briefBoardFixture {
	t.Helper()
	raw := mustReadRepoFile(t, "contracts", "brief-board-cases.json")
	var fx briefBoardFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", briefBoardFixtureRel, err)
	}
	for _, key := range briefBoardOwnerKeys {
		if fx.Owners[key] == "" {
			t.Fatalf("owners must name %q; got %v", key, fx.Owners)
		}
	}
	return fx
}

// ── the document model ──────────────────────────────────────────────────────

func TestBriefBoardEntryContractCases(t *testing.T) {
	fx := loadBriefBoardFixture(t)
	var tally sweepguard.Tally
	for _, c := range fx.Entries {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
			doc := parseBriefDoc(c.Brief)
			if len(doc.Entries) != len(c.Expect) {
				t.Fatalf("parsed %d entries, want %d\n  got:  %+v\n  why: %s", len(doc.Entries), len(c.Expect), doc.Entries, c.Why)
			}
			for i, want := range c.Expect {
				got := doc.Entries[i]
				if got.Column != want.Column || got.Group != want.Group || got.Start != want.Start || got.End != want.End {
					t.Errorf("entry %d = {column:%q group:%q start:%d end:%d}, want {column:%q group:%q start:%d end:%d}\n  why: %s",
						i, got.Column, got.Group, got.Start, got.End, want.Column, want.Group, want.Start, want.End, c.Why)
				}
				if strings.Join(got.Lines, "\n") != strings.Join(want.Lines, "\n") {
					t.Errorf("entry %d lines = %q, want %q\n  why: %s", i, got.Lines, want.Lines, c.Why)
				}
			}
			// The round-trip property, asserted on every case rather than once:
			// an archive move splices against these indexes, so a parse that
			// lost or reflowed a line would corrupt the user's brief.
			if back := strings.Join(doc.Lines, "\n"); back != c.Brief {
				t.Errorf("serialize(parse(brief)) is not byte-exact\n got %q\nwant %q", back, c.Brief)
			}
		})
	}
	if err := tally.RequireEvery("the brief entry corpus", briefEntryCorpusFloor); err != nil {
		t.Fatal(err)
	}
}

// ── the archive move ────────────────────────────────────────────────────────

func TestBriefArchiveContractCases(t *testing.T) {
	fx := loadBriefBoardFixture(t)
	var tally sweepguard.Tally
	for _, c := range fx.Archive {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
			dir := t.TempDir()
			briefPath := briefPathFor(dir)
			archivePath := briefArchivePathFor(dir)
			if err := os.MkdirAll(filepath.Dir(briefPath), 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(briefPath, []byte(c.Brief), 0o644); err != nil {
				t.Fatal(err)
			}
			if c.Archive != "" {
				if err := os.WriteFile(archivePath, []byte(c.Archive), 0o644); err != nil {
					t.Fatal(err)
				}
			}

			count, err := wholeBriefBound("count", c.Count)
			if err != nil {
				t.Fatal(err)
			}
			keep, err := wholeBriefBound("keep", c.Keep)
			if err != nil {
				t.Fatal(err)
			}
			res, err := archiveOldestEntries(dir, c.Section, count, keep, c.Date)
			if err != nil {
				t.Fatalf("archiveOldestEntries: %v\n  why: %s", err, c.Why)
			}

			gotBrief, err := os.ReadFile(briefPath)
			if err != nil {
				t.Fatal(err)
			}
			if string(gotBrief) != c.Expect.Brief {
				t.Errorf("brief after the move\n got %q\nwant %q\n  why: %s", gotBrief, c.Expect.Brief, c.Why)
			}
			gotArchive := ""
			if data, err := os.ReadFile(archivePath); err == nil {
				gotArchive = string(data)
			} else if !os.IsNotExist(err) {
				t.Fatal(err)
			}
			// "" means the file must not exist: a move that wrote nothing must
			// not leave an empty archive beside the brief either.
			if gotArchive != c.Expect.Archive {
				t.Errorf("archive after the move\n got %q\nwant %q\n  why: %s", gotArchive, c.Expect.Archive, c.Why)
			}
			if res.Archived != c.Expect.Archived {
				t.Errorf("archived = %d, want %d\n  why: %s", res.Archived, c.Expect.Archived, c.Why)
			}
			if res.EntriesInSection != c.Expect.EntriesInSection ||
				res.BytesInSection != c.Expect.BytesInSection ||
				res.BytesInBrief != c.Expect.BytesInBrief {
				t.Errorf("size report = {entries:%d bytesInSection:%d bytesInBrief:%d}, want {entries:%d bytesInSection:%d bytesInBrief:%d}\n  why: %s",
					res.EntriesInSection, res.BytesInSection, res.BytesInBrief,
					c.Expect.EntriesInSection, c.Expect.BytesInSection, c.Expect.BytesInBrief, c.Why)
			}
			if res.Date != c.Date || res.Section != c.Section || res.Path != briefPath || res.ArchivePath != archivePath {
				t.Errorf("result envelope = %+v, want the case's own section/date and the composed paths", res)
			}
		})
	}
	if err := tally.RequireEvery("the brief archive corpus", briefArchiveCorpusFloor); err != nil {
		t.Fatal(err)
	}
}

// ── the checker ─────────────────────────────────────────────────────────────

func TestBriefCheckContractCases(t *testing.T) {
	fx := loadBriefBoardFixture(t)
	var tally sweepguard.Tally
	for _, c := range fx.Check {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
			rep := checkNowSection(c.Brief, liveSessionIDs(c.Sessions), "/p/.workspacer/brief.md")
			if rep.EntriesChecked != c.Expect.EntriesChecked ||
				rep.EntriesLive != c.Expect.EntriesLive ||
				rep.LiveSessions != c.Expect.LiveSessions {
				t.Errorf("counts = {checked:%d live:%d sessions:%d}, want {checked:%d live:%d sessions:%d}\n  why: %s",
					rep.EntriesChecked, rep.EntriesLive, rep.LiveSessions,
					c.Expect.EntriesChecked, c.Expect.EntriesLive, c.Expect.LiveSessions, c.Why)
			}
			if len(rep.Findings) != len(c.Expect.Findings) {
				t.Fatalf("%d findings %+v, want %d %+v\n  why: %s", len(rep.Findings), rep.Findings, len(c.Expect.Findings), c.Expect.Findings, c.Why)
			}
			for i, want := range c.Expect.Findings {
				got := rep.Findings[i]
				if got.Line != want.Line || got.Text != want.Text || got.Reason != want.Reason ||
					strings.Join(got.Refs, ",") != strings.Join(want.Refs, ",") {
					t.Errorf("finding %d = {line:%d text:%q reason:%q refs:%v}, want {line:%d text:%q reason:%q refs:%v}\n  why: %s",
						i, got.Line, got.Text, got.Reason, got.Refs, want.Line, want.Text, want.Reason, want.Refs, c.Why)
				}
				// Every finding says what to do about it. A blank detail is a
				// finding a manager cannot act on.
				if strings.TrimSpace(got.Detail) == "" {
					t.Errorf("finding %d has no detail — the report IS the product here", i)
				}
			}
			if rep.Section != briefCheckedSection || rep.Path != "/p/.workspacer/brief.md" {
				t.Errorf("report envelope = {section:%q path:%q}, want {section:%q path:%q}", rep.Section, rep.Path, briefCheckedSection, "/p/.workspacer/brief.md")
			}
			if rep.UnavailableChecks != nil {
				t.Errorf("unavailableChecks = %v on a run with a real liveness source — the field must be ABSENT when every arm ran, or the desktop twin's shape is not what a caller gets", rep.UnavailableChecks)
			}
		})
	}
	if err := tally.RequireEvery("the brief check corpus", briefCheckCorpusFloor); err != nil {
		t.Fatal(err)
	}
}

// ── the wording, which the fixture deliberately does not carry ──────────────

// The report is prose a MODEL reads and acts on, so the two providers' sentences
// must not diverge: a manager told "this check does not edit the brief" by one
// provider and something else by the other learns a different rule about what
// its own tools do. The fixture pins the STRUCTURE; this pins the sentences,
// against briefCheck.ts itself rather than against a comment claiming they
// match. (Same technique as TestFleetMessageTwinMatchesTheDesktop.)
func TestBriefCheckWordingMatchesTheDesktop(t *testing.T) {
	ts := joinTSStringLiterals(string(mustReadRepoFile(t, "apps", "desktop", "src", "main", "services", "briefCheck.ts")))
	// A guard against the join-stripper silently matching nothing.
	if !strings.Contains(ts, "is not a session id, so it links to ") {
		t.Fatal("joinTSStringLiterals no longer recovers briefCheck.ts's literals — this twin check is comparing against mush")
	}

	rep := checkNowSection(
		"## Now\n- Dispatched it (session:deadbeef) — running.\n- Dispatched round 2 (session:6a-round2) — running.\n- Dispatched something with no id at all.\n",
		nil, "/p/.workspacer/brief.md")
	if len(rep.Findings) != 3 {
		t.Fatalf("expected one finding of each reason; got %d: %+v", len(rep.Findings), rep.Findings)
	}
	seen := map[string]bool{}
	for _, f := range rep.Findings {
		seen[f.Reason] = true
		// The composed halves (`session:deadbeef, `) are this side's; what has
		// to match is the SENTENCE the twin wrote, so the comparison is on the
		// invariant tail of each detail.
		tail := f.Detail
		if i := strings.Index(tail, "is not a session"); i > 0 {
			tail = tail[i:]
		}
		if !strings.Contains(ts, tail) {
			t.Errorf("the %q finding's wording has drifted from briefCheck.ts.\n  go: %q\n"+
				"A manager reads this sentence to decide what to do with the entry, and two providers "+
				"telling it different things about its own tools is worse than either sentence alone.", f.Reason, tail)
		}
	}
	for _, want := range []string{"stale", "malformed", "unreferenced"} {
		if !seen[want] {
			t.Errorf("the wording sweep never produced a %q finding — it is checking less than it claims", want)
		}
	}

	// And the two notes, which are what a manager reads FIRST.
	clean := checkNowSection("## Now\n- an ordinary standing note.\n", nil, "/p/b.md").Note
	if !strings.Contains(ts, "either names a session this host still ") {
		t.Error("the clean note's wording is not in briefCheck.ts any more")
	}
	if !strings.Contains(clean, "Nothing to prune.") {
		t.Errorf("clean note = %q, want the twin's 'Nothing to prune.' close", clean)
	}
	if !strings.Contains(ts, "This check only reports: it never ") {
		t.Error("the findings note's wording is not in briefCheck.ts any more")
	}
}

// ── this side's own behaviour ───────────────────────────────────────────────

// The check must not be able to change anything. The doctrine sentence in every
// header ("it flags, it never touches the file") is worth exactly as much as the
// test that drives the real capability over a brief and compares the bytes.
func TestBriefCheckNeverWritesToTheBrief(t *testing.T) {
	fx := newBriefFixture(t)
	brief := "## Now\n- Dispatched the thing (session:deadbeef) — running.\n"
	writeBrief(t, fx.agentCwd, brief)
	archivePath := briefArchivePathFor(fx.agentCwd)

	reg := registryWithCwds(t, fx.agentCwd)
	if _, err := reg.handle(context.Background(), "brief.check", mustJSON(t, map[string]any{"project": fx.agentCwd})); err != nil {
		t.Fatalf("brief.check: %v", err)
	}
	after, err := os.ReadFile(briefPathFor(fx.agentCwd))
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != brief {
		t.Errorf("brief.check changed the brief\n got %q\nwant %q", after, brief)
	}
	if _, err := os.Stat(archivePath); !os.IsNotExist(err) {
		t.Errorf("brief.check created %s — it is READ-ONLY and must leave no trace at all", archivePath)
	}
	if _, err := os.Stat(briefPathFor(fx.agentCwd) + lockFileSuffix); !os.IsNotExist(err) {
		t.Error("brief.check left a lock file behind")
	}
}

// A project with no brief is not an error: it has no stale Now lines, which is
// the honest answer and the one a manager can act on.
func TestBriefCheckOnAProjectWithNoBriefIsAnEmptyReport(t *testing.T) {
	fx := newBriefFixture(t)
	reg := registryWithCwds(t, fx.agentCwd)
	raw, err := reg.handle(context.Background(), "brief.check", mustJSON(t, map[string]any{"project": fx.agentCwd}))
	if err != nil {
		t.Fatalf("brief.check: %v", err)
	}
	var rep briefCheckReport
	if err := json.Unmarshal(raw, &rep); err != nil {
		t.Fatal(err)
	}
	if rep.EntriesChecked != 0 || len(rep.Findings) != 0 {
		t.Errorf("report = %+v, want an empty one", rep)
	}
	if _, err := os.Stat(briefPathFor(fx.agentCwd)); !os.IsNotExist(err) {
		t.Error("brief.check created the brief it was asked about")
	}
}

// WITH NO LIVENESS SOURCE THE REPORT SAYS SO, rather than reporting every
// referenced entry as stale — which is what an empty live set would otherwise
// produce, and it would be a lie in the direction that makes the tool useless
// (a checker that cries wolf is one a manager stops reading).
func TestBriefCheckDeclaresTheStaleArmWhenItHasNoSessionStore(t *testing.T) {
	fx := newBriefFixture(t)
	writeBrief(t, fx.agentCwd, "## Now\n- Dispatched A (session:deadbeef) — running.\n"+
		"- Dispatched B (session:6a-round2) — running.\n- Dispatched C with no id.\n")

	// A brain with no live session store is this brain in catalog scope. The
	// roots come from the agent-cwd cache, which the first call warms — so the
	// store is dropped between the two calls rather than never set, and a cache
	// that expired shows up as a containment refusal below, loudly, instead of
	// as a quietly different assertion.
	reg := registryWithCwds(t, fx.agentCwd)
	if _, err := reg.handle(context.Background(), "brief.check", mustJSON(t, map[string]any{"project": fx.agentCwd})); err != nil {
		t.Fatalf("warming brief.check: %v", err)
	}
	reg.store = nil
	raw, err := reg.handle(context.Background(), "brief.check", mustJSON(t, map[string]any{"project": fx.agentCwd}))
	if err != nil {
		t.Fatalf("brief.check with no store: %v", err)
	}
	var rep briefCheckReport
	if err := json.Unmarshal(raw, &rep); err != nil {
		t.Fatal(err)
	}
	if len(rep.UnavailableChecks) != 1 || rep.UnavailableChecks[0] != "stale" {
		t.Errorf("unavailableChecks = %v, want [stale] — a narrowed report that does not SAY it is narrowed reads as a clean one", rep.UnavailableChecks)
	}
	if rep.EntriesChecked != 3 {
		t.Errorf("entriesChecked = %d, want 3 — the entries were still read", rep.EntriesChecked)
	}
	for _, f := range rep.Findings {
		if f.Reason == "stale" {
			t.Errorf("reported a stale finding with no liveness source: %+v", f)
		}
	}
	// The two arms that never needed a session store still ran.
	reasons := map[string]bool{}
	for _, f := range rep.Findings {
		reasons[f.Reason] = true
	}
	if !reasons["malformed"] || !reasons["unreferenced"] {
		t.Errorf("findings = %+v, want the malformed and unreferenced arms to have run — they need no session store", rep.Findings)
	}
	if !strings.Contains(rep.Note, "ONE CHECK DID NOT RUN") {
		t.Errorf("note = %q, want it to name the check that did not run", rep.Note)
	}
}

// A FINISHED session counts as gone — driven through the real capability with a
// real store, which is the arm the fixture's pure cases cannot reach.
func TestBriefCheckReadsLivenessFromTheSessionStore(t *testing.T) {
	fx := newBriefFixture(t)
	writeBrief(t, fx.agentCwd, "## Now\n- Dispatched A (session:aaaaaaaa) — running.\n- Dispatched B (session:bbbbbbbb) — running.\n")

	// The two rows land in the SAME store the fixture's live agent is in — that
	// row is what makes fx.agentCwd a workspace root, so replacing the store
	// wholesale would take the containment guard's only root with it.
	reg := registryWithCwds(t, fx.agentCwd)
	reg.store.set("aaaaaaaa", json.RawMessage(`{"session_id":"aaaaaaaa","sessionId":"aaaaaaaa","mode":"input","status":"active"}`))
	reg.store.set("bbbbbbbb", json.RawMessage(`{"session_id":"bbbbbbbb","sessionId":"bbbbbbbb","mode":"stopped","status":"ended"}`))

	raw, err := reg.handle(context.Background(), "brief.check", mustJSON(t, map[string]any{"project": fx.agentCwd}))
	if err != nil {
		t.Fatalf("brief.check: %v", err)
	}
	var rep briefCheckReport
	if err := json.Unmarshal(raw, &rep); err != nil {
		t.Fatal(err)
	}
	if rep.UnavailableChecks != nil {
		t.Errorf("unavailableChecks = %v with a store present — every arm ran", rep.UnavailableChecks)
	}
	// Two live rows: the fixture's own agent (s0) and "aaaaaaaa". The ended one
	// is the point — it must not be counted.
	if rep.LiveSessions != 2 || rep.EntriesLive != 1 {
		t.Errorf("liveSessions=%d entriesLive=%d, want 2 and 1 — the ENDED row must not count as live", rep.LiveSessions, rep.EntriesLive)
	}
	if len(rep.Findings) != 1 || rep.Findings[0].Reason != "stale" || rep.Findings[0].Line != 2 {
		t.Errorf("findings = %+v, want one stale finding on line 2 (the ended worker's line)", rep.Findings)
	}
}

// A row that never went through the compat overlay carries only `session_id`.
// Reading it as id-less would flag a LIVE worker's Now line as stale, which is
// the one false positive this checker cannot afford.
func TestLiveSessionIDsFallsBackToTheSnakeCaseID(t *testing.T) {
	got := liveSessionIDs([]map[string]any{
		{"session_id": "AAAAAAAA", "mode": "input"},
		{"sessionId": "bbbbbbbb", "mode": "input"},
		{"session_id": "cccccccc", "mode": "stopped"},
		{"mode": "input"},
	})
	if strings.Join(got, ",") != "aaaaaaaa,bbbbbbbb" {
		t.Errorf("liveSessionIDs = %v, want [aaaaaaaa bbbbbbbb] — lower-cased, snake_case included, dead and id-less rows dropped", got)
	}
}

// ── brief.archive: this side's own behaviour ────────────────────────────────

// The caller must decide which question it is asking. Both or neither is a
// caller error, not a default.
func TestBriefArchiveRefusesBothOrNeitherBound(t *testing.T) {
	fx := newBriefFixture(t)
	writeBrief(t, fx.agentCwd, "## Now\n- one\n")
	reg := registryWithCwds(t, fx.agentCwd)
	for _, params := range []map[string]any{
		{"project": fx.agentCwd, "section": "Now"},
		{"project": fx.agentCwd, "section": "Now", "count": 1, "keep": 1},
	} {
		_, err := reg.handle(context.Background(), "brief.archive", mustJSON(t, params))
		if err == nil || !strings.Contains(err.Error(), "give either count") {
			t.Errorf("brief.archive(%v) = %v, want the either/or refusal", params, err)
		}
	}
	// And a fractional bound is refused rather than truncated: a caller that
	// meant 1.5 has not counted anything, and 1 would be a guess.
	_, err := reg.handle(context.Background(), "brief.archive", mustJSON(t, map[string]any{
		"project": fx.agentCwd, "section": "Now", "count": 1.5,
	}))
	if err == nil || !strings.Contains(err.Error(), "whole number") {
		t.Errorf("a fractional count = %v, want the whole-number refusal", err)
	}
}

// A section the brief does not have is a refusal, not a silent no-op: a manager
// that typed `## Landed` must learn its brief has no such heading rather than
// being told nothing moved.
func TestBriefArchiveRefusesASectionTheBriefDoesNotHave(t *testing.T) {
	fx := newBriefFixture(t)
	writeBrief(t, fx.agentCwd, "## Now\n- one\n")
	reg := registryWithCwds(t, fx.agentCwd)
	_, err := reg.handle(context.Background(), "brief.archive", mustJSON(t, map[string]any{
		"project": fx.agentCwd, "section": "Direction", "count": 1,
	}))
	if err == nil || !strings.Contains(err.Error(), "no \"## Direction\" section") {
		t.Errorf("brief.archive on an absent section = %v, want the missing-heading refusal", err)
	}
	// A section name that is not a section AT ALL is refused earlier, by the
	// same parse brief.append uses — before any file is touched.
	_, err = reg.handle(context.Background(), "brief.archive", mustJSON(t, map[string]any{
		"project": fx.agentCwd, "section": "Landed", "count": 1,
	}))
	if err == nil || !strings.Contains(err.Error(), "unknown section") {
		t.Errorf("brief.archive on an unknown section = %v, want the section refusal", err)
	}
}

// `keep` is the idempotent form, and that is the property /checkpoint relies on:
// running it twice must move nothing the second time and must not rewrite the
// archive either.
func TestBriefArchiveKeepIsIdempotent(t *testing.T) {
	fx := newBriefFixture(t)
	writeBrief(t, fx.agentCwd, "## Recently\n- newest\n- middle\n- oldest\n")
	reg := registryWithCwds(t, fx.agentCwd)
	params := mustJSON(t, map[string]any{"project": fx.agentCwd, "section": "Recently", "keep": 1})

	first, err := reg.handle(context.Background(), "brief.archive", params)
	if err != nil {
		t.Fatalf("first archive: %v", err)
	}
	briefAfterFirst := mustRead(t, briefPathFor(fx.agentCwd))
	archiveAfterFirst := mustRead(t, briefArchivePathFor(fx.agentCwd))
	var res briefArchiveResult
	if err := json.Unmarshal(first, &res); err != nil {
		t.Fatal(err)
	}
	if res.Archived != 2 {
		t.Fatalf("first run archived %d, want 2", res.Archived)
	}

	second, err := reg.handle(context.Background(), "brief.archive", params)
	if err != nil {
		t.Fatalf("second archive: %v", err)
	}
	if err := json.Unmarshal(second, &res); err != nil {
		t.Fatal(err)
	}
	if res.Archived != 0 {
		t.Errorf("second run archived %d, want 0 — `keep` is the idempotent form", res.Archived)
	}
	if got := mustRead(t, briefPathFor(fx.agentCwd)); got != briefAfterFirst {
		t.Errorf("the second run rewrote the brief\n got %q\nwant %q", got, briefAfterFirst)
	}
	if got := mustRead(t, briefArchivePathFor(fx.agentCwd)); got != archiveAfterFirst {
		t.Errorf("the second run rewrote the archive\n got %q\nwant %q", got, archiveAfterFirst)
	}
}

// NOTHING IS LOST. The move is the property the whole file turns on: every line
// that leaves the brief must be in the archive, byte for byte.
func TestBriefArchiveMovesEveryLineItRemoves(t *testing.T) {
	fx := newBriefFixture(t)
	brief := "## Now\n- first\n  with a continuation\n- second\n\n## Direction\n- keep me\n"
	writeBrief(t, fx.agentCwd, brief)
	reg := registryWithCwds(t, fx.agentCwd)
	if _, err := reg.handle(context.Background(), "brief.archive", mustJSON(t, map[string]any{
		"project": fx.agentCwd, "section": "Now", "count": 2,
	})); err != nil {
		t.Fatalf("brief.archive: %v", err)
	}
	after := mustRead(t, briefPathFor(fx.agentCwd))
	archive := mustRead(t, briefArchivePathFor(fx.agentCwd))
	for _, line := range []string{"- first", "  with a continuation", "- second"} {
		if strings.Contains(after, line) {
			t.Errorf("%q is still in the brief", line)
		}
		if !strings.Contains(archive, line) {
			t.Errorf("%q left the brief and is not in the archive — that is the one damage this must never do", line)
		}
	}
	if !strings.Contains(after, "- keep me") {
		t.Error("the move touched another section")
	}
}

// The compare-and-swap, driven the only way it is observable: change the file
// between the two reads. A writer that beat us must not be clobbered, and — the
// half that matters more for a MOVE — the entry must not be archived twice by
// the retry.
func TestBriefArchiveRetriesAgainstAnOutsideWriterWithoutDuplicating(t *testing.T) {
	fx := newBriefFixture(t)
	briefPath := briefPathFor(fx.agentCwd)
	writeBrief(t, fx.agentCwd, "## Now\n- oldest\n- newest\n")

	// One interposition, on the first attempt only: an agent's Edit tool
	// appending a line while we compute.
	fired := false
	briefCASHook = func(p string) {
		if fired || p != briefPath {
			return
		}
		fired = true
		if err := os.WriteFile(briefPath, []byte("## Now\n- oldest\n- newest\n- an outside writer's line\n"), 0o644); err != nil {
			t.Error(err)
		}
	}
	t.Cleanup(func() { briefCASHook = func(string) {} })

	reg := registryWithCwds(t, fx.agentCwd)
	if _, err := reg.handle(context.Background(), "brief.archive", mustJSON(t, map[string]any{
		"project": fx.agentCwd, "section": "Now", "count": 1,
	})); err != nil {
		t.Fatalf("brief.archive: %v", err)
	}
	if !fired {
		t.Fatal("the CAS seam never fired — this test did not exercise the retry path")
	}
	after := mustRead(t, briefPathFor(fx.agentCwd))
	if !strings.Contains(after, "- an outside writer's line") {
		t.Errorf("the outside writer's line was clobbered: %q", after)
	}
	if strings.Contains(after, "- oldest") {
		t.Errorf("the archived entry is still in the brief: %q", after)
	}
	archive := mustRead(t, briefArchivePathFor(fx.agentCwd))
	if n := strings.Count(archive, "- oldest"); n != 1 {
		t.Errorf("the archive holds %d copies of the moved entry, want 1 — a side effect performed before the CAS passes is repeated by every retry:\n%s", n, archive)
	}
}

func mustJSON(t *testing.T, v any) json.RawMessage {
	t.Helper()
	body, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return json.RawMessage(body)
}

func writeBrief(t *testing.T, dir, content string) {
	t.Helper()
	p := briefPathFor(dir)
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func mustRead(t *testing.T, p string) string {
	t.Helper()
	data, err := os.ReadFile(p)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}
