package main

// Tests for the headless-provider port: the fleet verbs (agentops.go), the
// brief writer (brief.go), the image preview (readimage.go), the recent-session
// list (recent.go), the visible-terminal request (visibleterm.go), and the
// wake-format twin (fleetmsg.go).
//
// The confinement cases live in briefconfinement_test.go beside the corpus
// sweep they extend; this file is behaviour and contract.

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// ── tier ────────────────────────────────────────────────────────────────────

// THE TIER CHECK, MEASURED RATHER THAN ASSUMED.
//
// Every capability this port adds is either already admitted to a scoped tier
// or operator-only by construction, and the difference matters: agents.reparent
// re-points another manager's workers and terminals.open runs a visible host
// process. Registering a method in the brain does not change its tier — the
// authtoken allowlists are provider-agnostic — but "it does not change" is
// exactly the kind of claim that is true until someone edits the allowlist for
// an unrelated reason, so it is measured here.
func TestHeadlessPortAddsNothingToTheScopedTiers(t *testing.T) {
	wantScoped := portedMethodStanding()

	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage} {
		allowed := map[string]bool{}
		for _, m := range scope.Methods() {
			if m == "*" {
				t.Fatalf("%s resolved to a wildcard — the scoped tiers are exact-name allowlists and this guard is measuring nothing", scope)
			}
			allowed[m] = true
		}
		if len(allowed) < 10 {
			t.Fatalf("%s enumerated only %d methods — the allowlist moved and this guard is guarding nothing", scope, len(allowed))
		}
		for method, scopedOK := range wantScoped {
			switch {
			case allowed[method] && !scopedOK:
				t.Errorf("%s now grants %q, which this port registered as an operator-only capability. "+
					"If that admission is deliberate, say why in authtoken.go and update this guard; if it is not, "+
					"a phone or plugin token can now reach it.", scope, method)
			case !allowed[method] && scopedOK && scope == authtoken.ScopeView:
				t.Errorf("%s no longer grants %q, which this port assumed was already admitted — "+
					"the headless provider for it is now unreachable from the tier that needs it", scope, method)
			}
		}
	}
}

// EVERY BRIEF VERB THE FACADE EXPOSES HAS A BRAIN HANDLER — the guard the map
// above could not be, and the gap it was written a verb too early to catch.
//
// The map is a TIER check: it says what each ported method's standing is
// supposed to be, and it is silent about a method that was never ported. That is
// exactly how brief.check and brief.archive went missing. capspec declared all
// three brief verbs, cmd/mcp exposed all three as MCP tools, and the brain
// registered ONE — so a Fleet Manager on a headless node called brief_check,
// got `no provider for brief.check` from the bus, and lost two of the three
// tools its own doctrine tells it to use. Nothing was red.
//
// So the enumeration runs from the two places a brief verb is DECLARED — the
// MCP facade's tool table (what an agent can actually reach) and capspec's
// path-param registry (what the bus knows about it) — rather than from a list
// here, which would be the same hand-maintained thing that failed. A fourth
// brief verb added to either one without a brain handler fails this test.
func TestEveryBriefVerbTheFacadeExposesHasABrainHandler(t *testing.T) {
	declared := map[string]string{}
	facade := string(mustReadRepoFile(t, "services", "hub", "cmd", "mcp", "main.go"))
	for _, m := range briefMethodRe.FindAllStringSubmatch(facade, -1) {
		declared[m[1]] = "the MCP facade exposes it as a tool (cmd/mcp/main.go)"
	}
	if len(declared) < 3 {
		t.Fatalf("parsed %d brief.* methods from cmd/mcp/main.go (%v) — the registration syntax changed and this guard is enumerating nothing", len(declared), declared)
	}
	for method := range capspec.PathParam {
		if strings.HasPrefix(method, "brief.") {
			if _, seen := declared[method]; !seen {
				declared[method] = "capspec declares it path-scoped (internal/capspec/capspec.go)"
			}
		}
	}

	reg := newRegistry(newClaudemonClient("http://unused"))
	served := map[string]bool{}
	for _, set := range [][]string{reg.methods(), reg.catalogMethods()} {
		for _, m := range set {
			served[m] = true
		}
	}

	for method, why := range declared {
		if !served[method] {
			t.Errorf("%s, and NO brain handler answers it. On a headless node that is `no provider for %s` at the bus level — "+
				"silent, and it takes a tool the Fleet Manager doctrine tells the manager to use. Register it in handlers.go "+
				"(and dispatch it), or, if it is genuinely desktop-only, say why in cmd/brain/brief.go's header the way the "+
				"first port did.", why, method)
			continue
		}
		if _, standing := portedMethodStanding()[method]; !standing {
			t.Errorf("the brain serves %s and TestHeadlessPortAddsNothingToTheScopedTiers says nothing about its tier — "+
				"add it to wantScoped, or a widened view/triage allowlist would hand a phone or plugin token a capability "+
				"that writes to a caller-chosen project directory with nobody noticing", method)
		}
	}
}

// briefMethodRe finds the capability names the MCP facade forwards to, narrowed
// to the brief family. The facade writes them as bare string literals on the
// addTool call, which is why a literal match is enough here.
var briefMethodRe = regexp.MustCompile(`"(brief\.[a-zA-Z][\w.]*)"`)

// portedMethodStanding is the port's methods and what each one's tier standing
// is SUPPOSED to be. ONE literal, read by both tests above: the tier check asks
// whether the allowlists still agree with it, and the completeness check asks
// whether a served brief verb appears in it at all. A second copy would be the
// same hand-maintained list drifting from itself.
func portedMethodStanding() map[string]bool {
	return map[string]bool{
		// Already in viewMethods before this port, admitted there because it
		// names no recipient and cannot name a sender. Nothing here widens it.
		"agents.reportProgress": true,
		// Already in viewMethods: a numeric limit over rows the provider holds.
		"sessions.recent": true,

		// Operator-only by construction — in NEITHER tier's exact-name
		// allowlist. Each is here because it is privileged, not because it was
		// overlooked:
		//   agents.reparent  re-points another manager's live workers
		//   agents.close     removes a session row (and SIGTERMs a live one)
		//   agents.orphans   discloses dead managers' labels/cwds
		//   agents.notifyWhen arms a wake against another session
		//   brief.append     writes to a caller-chosen project directory
		//   brief.archive    MOVES entries out of one, into the file beside it
		//   brief.check      reads one, and reads the session store to judge it
		//   terminals.open   runs a command in a VISIBLE host shell
		//   fs.readImage     reads host file bytes
		//   claude.set*      changes a running agent's model/effort/approvals
		"agents.reparent":          false,
		"agents.close":             false,
		"agents.orphans":           false,
		"agents.notifyWhen":        false,
		"brief.append":             false,
		"brief.archive":            false,
		"brief.check":              false,
		"terminals.open":           false,
		"fs.readImage":             false,
		"claude.setPermissionMode": false,
		"claude.setModel":          false,
		"claude.setEffort":         false,
		"claude.handoffBrief":      false,
	}
}

// ── the fleet-wake format twin ──────────────────────────────────────────────

// The wake text IS the channel: it lands in the manager's transcript as an
// ordinary user turn, and the GUI re-parses that text back into a card. So the
// Go strings are pinned to the TypeScript source rather than to a comment
// claiming they match.
func TestFleetMessageTwinMatchesTheDesktop(t *testing.T) {
	ts := joinTSStringLiterals(string(mustReadRepoFile(t, "apps", "desktop", "src", "main", "shared", "fleetMessages.ts")))
	for name, want := range map[string]string{
		"progress header":        fleetProgressHeader,
		"threshold header":       fleetThresholdHeader,
		"progress tail":          fleetProgressTail,
		"threshold tail":         fleetThresholdTail,
		"worker-finished header": fleetWorkerFinishedHeader,
		"worker-FAILED header":   fleetWorkerFailedHeader,
		"catch-up header":        fleetCatchUpHeader,
		"worker-finished tail":   fleetWorkerFinishedTail,
		"catch-up tail":          fleetCatchUpTail,
		// The blocked broadcast (blockwake.go). Its header is the ONE that opens
		// `[supervisor]` rather than `[fleet]`, which is exactly the kind of
		// detail a re-spelling would "tidy" — and every client's card parser
		// keys off the literal.
		"blocked header":          fleetBlockedHeader,
		"blocked tail":            fleetBlockedTail,
		"failed note":             fleetFailedNote,
		"stopped note":            fleetStoppedNote,
		"full-reply block prefix": "Full final message — ",
		// agents.sendMessage's attribution header. Split into its two fixed
		// halves on both sides precisely so it can be pinned here: the
		// TypeScript composes it with a template literal, which this
		// source-text comparison cannot see through.
		"sender header prefix": fleetSenderHeaderPrefix,
		"sender header suffix": fleetSenderHeaderSuffix,
	} {
		if !strings.Contains(ts, want) {
			t.Errorf("%s has drifted from fleetMessages.ts.\n  go: %q\n"+
				"A header the parser does not recognize silently demotes every wake card to a text blob on every client, "+
				"and a tail the manager was not trained on changes what it does with the wake.", name, want)
		}
	}
	// And the comparison must not be vacuous: if the join-stripper stops
	// working, `Contains` starts matching nothing and every case above passes
	// only because it never ran against real text.
	if !strings.Contains(ts, "[fleet] Worker finished:") {
		t.Fatal("joinTSStringLiterals no longer recovers fleetMessages.ts's literals — this twin check is comparing against mush")
	}
}

// joinTSStringLiterals reconstructs TypeScript's adjacent-literal concatenation
// (`'a ' +\n  'b'` → `'a b'`) so a Go constant can be compared against the
// source text it was ported from. The TS wraps these strings at arbitrary
// points, so nothing else would survive a reformat of that file.
var tsLiteralJoinRe = regexp.MustCompile("[`'\"]\\s*\\+\\s*[`'\"]")

func joinTSStringLiterals(src string) string {
	return tsLiteralJoinRe.ReplaceAllString(src, "")
}

// The bullet grammar's optional tails must appear in the order the desktop's
// ENTRY_RE matches them, or the bullet is unparseable rather than merely
// differently worded.
func TestFleetEntryBulletGrammar(t *testing.T) {
	got := formatFleetEntry(fleetEntry{Label: "worker", SessionID: "s1", Cwd: "/p", Note: "phase 1 done", NeedsDecision: true})
	want := "worker (session:s1, cwd /p) — NEEDS A DECISION — reports: phase 1 done"
	if got != want {
		t.Errorf("progress bullet\n got %q\nwant %q", got, want)
	}
	got = formatFleetEntry(fleetEntry{Label: "worker", SessionID: "s1", Cwd: "", Crossed: "tokens 1,000 ≥ 500"})
	want = "worker (session:s1, cwd ?) — crossed: tokens 1,000 ≥ 500"
	if got != want {
		t.Errorf("threshold bullet\n got %q\nwant %q", got, want)
	}
	// A blocked bullet spends the `where` slot on the block kind INSTEAD of a
	// cwd — ENTRY_RE's `(?:cwd (.+?)|(approval|question))` is an alternation,
	// so emitting both (or defaulting to "cwd ?") makes the bullet unparseable
	// and the card degrades to a text blob on every client.
	got = formatFleetEntry(fleetEntry{Label: "worker", SessionID: "s1", Cwd: "/p", BlockedOn: "approval"})
	want = "worker (session:s1, approval)"
	if got != want {
		t.Errorf("blocked bullet\n got %q\nwant %q", got, want)
	}
	got = formatFleetEntry(fleetEntry{Label: "worker", SessionID: "s1", BlockedOn: "question"})
	want = "worker (session:s1, question)"
	if got != want {
		t.Errorf("blocked bullet (question)\n got %q\nwant %q", got, want)
	}
}

// The composed sender header, byte-for-byte. The desktop asserts the SAME two
// strings (fleetMessages.test.ts, and hubCapabilities.test.ts on what the
// recipient actually receives) — a divergence here means a worker's message
// reads differently to its manager depending on which host delivered it.
func TestFleetSenderHeaderText(t *testing.T) {
	if got, want := fleetSenderHeaderText("worker1", "Rust Worker"), "[fleet] session:worker1 (Rust Worker) says:\n"; got != want {
		t.Errorf("labelled sender\n got %q\nwant %q", got, want)
	}
	if got, want := fleetSenderHeaderText("worker2", ""), "[fleet] session:worker2 says:\n"; got != want {
		t.Errorf("unlabelled sender\n got %q\nwant %q", got, want)
	}
}

func TestGroupThousandsMatchesToLocaleString(t *testing.T) {
	for _, c := range []struct {
		in   float64
		want string
	}{{0, "0"}, {999, "999"}, {1000, "1,000"}, {309412, "309,412"}, {1234567, "1,234,567"}} {
		if got := groupThousands(c.in); got != c.want {
			t.Errorf("groupThousands(%v) = %q, want %q", c.in, got, c.want)
		}
	}
}

// ── agents.reportProgress ───────────────────────────────────────────────────

// fleetReg builds a full-scope registry with a live store seeded from raw
// claudemon rows and the given spawn metadata, so the fleet verbs see the same
// enriched view they see in production.
func fleetReg(t *testing.T, srvURL string, rows map[string]string, meta map[string]spawnMeta) *registry {
	t.Helper()
	reg := newRegistry(newClaudemonClient(srvURL))
	m := newMetaStore()
	for id, sm := range meta {
		m.set(id, sm)
	}
	reg.meta = m
	store := newSessionStore()
	store.enrich = func(snap json.RawMessage) json.RawMessage { return enrichAndCompat(snap, m) }
	for id, row := range rows {
		store.set(id, json.RawMessage(row))
	}
	reg.store = store
	return reg
}

// A live claudemon row. `mode` drives compatSnapshot's status/ambientState.
func row(id, cwd, mode string) string {
	return `{"session_id":"` + id + `","cwd":"` + cwd + `","mode":"` + mode + `"}`
}

func TestReportProgressRoutesOnlyToTheCallersOwnParent(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := fleetReg(t, srv.URL,
		map[string]string{
			"mgr":       row("mgr", "/work", "input"),
			"worker":    row("worker", "/work/proj", "responding"),
			"other-mgr": row("other-mgr", "/elsewhere", "input"),
		},
		map[string]spawnMeta{
			"mgr":    {Label: "Fleet Manager", IsWakeTarget: true},
			"worker": {Label: "rust worker", ParentSessionID: "mgr"},
		})

	res, err := reg.handle(context.Background(), "agents.reportProgress",
		json.RawMessage(`{"callerSessionId":"worker","note":"phase 1 landed","needsDecision":true}`))
	if err != nil {
		t.Fatal(err)
	}
	var out struct {
		DeliveredTo string `json:"deliveredTo"`
	}
	_ = json.Unmarshal(res, &out)
	if out.DeliveredTo != "mgr" {
		t.Fatalf("delivered to %q, want the caller's own parent", out.DeliveredTo)
	}
	// THE CONTAINMENT: the message went to the parent and to nobody else. A
	// recipient parameter would have let a worker wake any session on the host.
	if n := len(rec.calls("/sessions/other-mgr/message")); n != 0 {
		t.Errorf("a session that dispatched nothing received %d message(s)", n)
	}
	msgs := rec.calls("/sessions/mgr/message")
	if len(msgs) != 1 {
		t.Fatalf("expected one wake to the parent, got %d", len(msgs))
	}
	text, _ := msgs[0].body["text"].(string)
	if !strings.HasPrefix(text, fleetProgressHeader) {
		t.Errorf("wake does not open with the progress header (the GUI card parser keys off it):\n%s", text)
	}
	if !strings.Contains(text, "— NEEDS A DECISION — reports: phase 1 landed") {
		t.Errorf("wake bullet lost the caller's note or its decision flag:\n%s", text)
	}
	// The header must NOT read as a completion — a manager booking a progress
	// line as an outcome is this tool's one failure mode.
	if strings.Contains(fleetProgressHeader, "finished") {
		t.Error("the progress header contains the word 'finished'")
	}
}

func TestReportProgressRefusals(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := fleetReg(t, srv.URL,
		map[string]string{
			"mgr":      row("mgr", "/work", "input"),
			"dead-mgr": row("dead-mgr", "/work", "stopped"),
			"worker":   row("worker", "/work/p", "responding"),
			"orphan":   row("orphan", "/work/q", "responding"),
			"lonely":   row("lonely", "/work/r", "responding"),
		},
		map[string]spawnMeta{
			"worker": {ParentSessionID: "mgr"},
			"orphan": {ParentSessionID: "dead-mgr"},
		})
	ctx := context.Background()

	for _, c := range []struct {
		name, params, wantIn string
	}{
		{"no identity", `{"note":"hi"}`, "could not identify your session"},
		{"empty note", `{"callerSessionId":"worker","note":"   "}`, "non-empty note"},
		{"untracked caller", `{"callerSessionId":"nope","note":"hi"}`, "not a tracked session"},
		{"no parent", `{"callerSessionId":"lonely","note":"hi"}`, "you have no parent session"},
		{"dead parent", `{"callerSessionId":"orphan","note":"hi"}`, "has ended"},
		{"over the cap", `{"callerSessionId":"worker","note":"` + strings.Repeat("x", progressNoteMax+1) + `"}`, "the limit is 500"},
	} {
		t.Run(c.name, func(t *testing.T) {
			_, err := reg.handle(ctx, "agents.reportProgress", json.RawMessage(c.params))
			if err == nil {
				t.Fatalf("accepted %s", c.name)
			}
			if !strings.Contains(err.Error(), c.wantIn) {
				t.Errorf("refusal %q does not contain %q", err, c.wantIn)
			}
		})
	}
	// Nothing above was delivered: every refusal is silent on the wire.
	if n := len(rec.calls("/sessions/mgr/message")); n != 0 {
		t.Errorf("%d refused report(s) still woke the manager", n)
	}
}

// The three budget refusals, which exist because an unsolicited wake interrupts
// a manager that is forbidden to poll. Each REFUSES OUT LOUD — a worker that
// believes it reported and did not is the failure the tool prevents.
func TestReportProgressBudgets(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := fleetReg(t, srv.URL,
		map[string]string{"mgr": row("mgr", "/w", "input"), "worker": row("worker", "/w/p", "responding")},
		map[string]spawnMeta{"worker": {ParentSessionID: "mgr"}})
	ctx := context.Background()

	send := func(note string) error {
		p, _ := json.Marshal(map[string]any{"callerSessionId": "worker", "note": note})
		_, err := reg.handle(ctx, "agents.reportProgress", p)
		return err
	}
	if err := send("first"); err != nil {
		t.Fatal(err)
	}
	if err := send("first"); err == nil || !strings.Contains(err.Error(), "same note") {
		t.Errorf("a duplicate note was not refused: %v", err)
	}
	if err := send("second"); err == nil || !strings.Contains(err.Error(), "limited to one per 60s") {
		t.Errorf("the rate limit did not hold: %v", err)
	}
	// The lifetime cap, reached by backdating the budget so the interval never
	// bites. The cap is charged BEFORE delivery, so a worker retrying a failing
	// send still reaches it.
	reg.progressMu.Lock()
	reg.progress["worker"] = progressBudget{count: progressMaxReports, lastAt: time.Now().Add(-time.Hour)}
	reg.progressMu.Unlock()
	if err := send("late"); err == nil || !strings.Contains(err.Error(), "which is the limit for one session") {
		t.Errorf("the lifetime cap did not hold: %v", err)
	}
}

// The bounds are the desktop's, and drift in either direction changes what a
// manager's context looks like.
func TestProgressBoundsMatchTheDesktop(t *testing.T) {
	ts := string(mustReadRepoFile(t, "apps", "desktop", "src", "main", "services", "progressReports.ts"))
	for _, c := range []struct{ decl, got string }{
		{"export const NOTE_MAX = ", "500"},
		{"export const MIN_INTERVAL_MS = ", "60_000"},
		{"export const MAX_REPORTS = ", "20"},
	} {
		if !strings.Contains(ts, c.decl+c.got) {
			t.Errorf("progressReports.ts no longer declares `%s%s` — the Go constants in agentops.go are now a second, different policy", c.decl, c.got)
		}
	}
	if progressNoteMax != 500 || progressMinInterval != 60*time.Second || progressMaxReports != 20 {
		t.Errorf("the Go bounds drifted: %d / %s / %d", progressNoteMax, progressMinInterval, progressMaxReports)
	}
}

// ── agents.notifyWhen ───────────────────────────────────────────────────────

func TestNotifyWhenArmsAndFires(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	// The token counters live on the RAW status_line, which is what
	// updateStatusLine refreshes — reading only the camel overlay would compare
	// against the numbers at the last full snapshot.
	worker := `{"session_id":"worker","cwd":"/w/p","mode":"responding","status_line":{"total_input_tokens":200000,"total_output_tokens":109412,"cost_usd":9.5}}`
	reg := fleetReg(t, srv.URL,
		map[string]string{"mgr": row("mgr", "/w", "input"), "worker": worker},
		map[string]spawnMeta{"mgr": {IsWakeTarget: true}, "worker": {Label: "rust worker", ParentSessionID: "mgr"}})
	ctx := context.Background()

	// No notifySessionId: the recipient defaults to the target's PARENT.
	res, err := reg.handle(ctx, "agents.notifyWhen", json.RawMessage(`{"sessionId":"worker","tokens":250000}`))
	if err != nil {
		t.Fatal(err)
	}
	var w thresholdWatch
	if err := json.Unmarshal(res, &w); err != nil {
		t.Fatal(err)
	}
	if w.WatcherSessionID != "mgr" {
		t.Fatalf("watcher defaulted to %q, want the target's parent", w.WatcherSessionID)
	}

	reg.sweepThresholds(ctx, time.Now())
	msgs := rec.calls("/sessions/mgr/message")
	if len(msgs) != 1 {
		t.Fatalf("expected one threshold wake, got %d", len(msgs))
	}
	text, _ := msgs[0].body["text"].(string)
	if !strings.HasPrefix(text, fleetThresholdHeader) {
		t.Errorf("wake does not open with the threshold header:\n%s", text)
	}
	if !strings.Contains(text, "crossed: tokens 309,412 ≥ 250,000") {
		t.Errorf("wake does not render the crossing the way the desktop does:\n%s", text)
	}
	// ONE-SHOT: gone before delivery, so a second sweep sends nothing.
	reg.sweepThresholds(ctx, time.Now())
	if n := len(rec.calls("/sessions/mgr/message")); n != 1 {
		t.Errorf("a one-shot watch fired %d times", n)
	}
}

func TestNotifyWhenRefusesAWatchThatCouldNeverFire(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := fleetReg(t, srv.URL,
		map[string]string{
			"mgr":    row("mgr", "/w", "input"),
			"worker": row("worker", "/w/p", "responding"),
			"dead":   row("dead", "/w/q", "stopped"),
		},
		map[string]spawnMeta{"worker": {ParentSessionID: "mgr"}, "dead": {ParentSessionID: "mgr"}})
	ctx := context.Background()

	for _, c := range []struct{ name, params, wantIn string }{
		{"no threshold", `{"sessionId":"worker"}`, "at least one threshold"},
		{"negative", `{"sessionId":"worker","usd":-1}`, "must be a positive number"},
		{"unknown target", `{"sessionId":"nope","tokens":1}`, "no such session"},
		{"ended target", `{"sessionId":"dead","tokens":1}`, "already ended"},
		{"no recipient", `{"sessionId":"mgr","tokens":1}`, "no notifySessionId and the target has no parent"},
		{"dead recipient", `{"sessionId":"worker","notifySessionId":"dead","tokens":1}`, "not a live session"},
	} {
		t.Run(c.name, func(t *testing.T) {
			if _, err := reg.handle(ctx, "agents.notifyWhen", json.RawMessage(c.params)); err == nil {
				t.Fatalf("armed %s — the caller now believes it is being watched and is not", c.name)
			} else if !strings.Contains(err.Error(), c.wantIn) {
				t.Errorf("refusal %q does not contain %q", err, c.wantIn)
			}
		})
	}
}

// The idle predicate must fire for a WEDGED session — one still claiming to
// work — because that is the failure it is armed for. Gating it on
// ambientState=="idle" made it structurally unfireable for exactly that case.
func TestNotifyWhenIdleFiresForAWedgedSession(t *testing.T) {
	stale := time.Now().Add(-10 * time.Minute).UnixMilli()
	s := fleetSession{SessionID: "w", AmbientState: "streaming", LastActivity: stale}
	idle := 60.0
	got := crossedBy(&thresholdWatch{IdleSeconds: &idle}, s, time.Now())
	if got == "" {
		t.Fatal("a session with no output for 10 minutes did not cross a 60s idle watch because it still claims to be streaming")
	}
	if !strings.Contains(got, "still reports streaming") {
		t.Errorf("the wake hides the claimed state, so the manager cannot tell a long tool call from a wedge: %q", got)
	}
}

// ── agents.close ────────────────────────────────────────────────────────────

func TestCloseRefusesAWorkingSessionAndTearsDownNothing(t *testing.T) {
	for _, ambient := range []string{"thinking", "streaming", "background"} {
		t.Run(ambient, func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			// mode "responding" → ambientState "streaming"; the background case
			// is a live session with a background task counted on the row.
			mode := "responding"
			extra := ""
			if ambient == "background" {
				mode, extra = "input", `,"background_tasks":2`
			}
			reg := fleetReg(t, srv.URL,
				map[string]string{"w": `{"session_id":"w","cwd":"/p","mode":"` + mode + `"` + extra + `}`}, nil)
			if ambient == "thinking" {
				t.Skip("claudemon's vocabulary has no mode that maps to `thinking`; streaming and background cover the live states this provider can see")
			}

			_, err := reg.handle(context.Background(), "agents.close", json.RawMessage(`{"sessionId":"w"}`))
			if err == nil {
				t.Fatal("closed a WORKING session — it would vanish from list_agents while it kept spending")
			}
			if !strings.Contains(err.Error(), "is still working") {
				t.Errorf("refusal does not say why: %v", err)
			}
			// Checked BEFORE any teardown: a refusal leaves the worker exactly
			// as it was.
			if n := len(rec.calls("/sessions/w/signal")); n != 0 {
				t.Errorf("a REFUSED close still signalled the daemon %d time(s)", n)
			}
			if _, still := reg.store.get("w"); !still {
				t.Error("a REFUSED close still removed the row")
			}
		})
	}
}

func TestCloseForgetsTheRowAndStopsALiveDaemonSession(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := fleetReg(t, srv.URL, map[string]string{
		"idle":  row("idle", "/p", "input"),
		"ended": row("ended", "/q", "stopped"),
	}, map[string]spawnMeta{"idle": {Label: "worker"}})
	ctx := context.Background()

	res, err := reg.handle(ctx, "agents.close", json.RawMessage(`{"sessionId":"idle"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out map[string]any
	_ = json.Unmarshal(res, &out)
	if out["removed"] != true || out["wasLive"] != true || out["daemon"] != "stopped" {
		t.Errorf("closing a live idle session reported %v", out)
	}
	if _, still := reg.store.get("idle"); still {
		// claudemon keeps the row as a resumable Stopped one, so if the brain
		// did not forget it too, the verb would do nothing visible at all.
		t.Error("the row survived the close")
	}
	if n := len(rec.calls("/sessions/idle/signal")); n != 1 {
		t.Errorf("expected one SIGTERM to the daemon, got %d", n)
	}

	// An ALREADY-ENDED row is forgotten without a second signal: re-SIGTERMing
	// it is the pointless call whose 404 this verb replaces.
	if _, err := reg.handle(ctx, "agents.close", json.RawMessage(`{"sessionId":"ended"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(rec.calls("/sessions/ended/signal")); n != 0 {
		t.Errorf("an ended session was signalled %d time(s)", n)
	}

	// And an unknown id is a no-op success, not an error — "it had already been
	// forgotten" is exactly what the caller asked for.
	res, err = reg.handle(ctx, "agents.close", json.RawMessage(`{"sessionId":"never-existed"}`))
	if err != nil {
		t.Fatalf("closing an unknown session errored: %v", err)
	}
	_ = json.Unmarshal(res, &out)
	if out["removed"] != false {
		t.Errorf("closing an unknown session reported %v", out)
	}
}

// ── agents.orphans / agents.reparent ────────────────────────────────────────

func TestOrphansReportsOnlyDeadParentsWithLiveChildren(t *testing.T) {
	reg := fleetReg(t, "http://127.0.0.1:1",
		map[string]string{
			"live-mgr":  row("live-mgr", "/a", "input"),
			"live-kid":  row("live-kid", "/a/x", "responding"),
			"dead-mgr":  row("dead-mgr", "/b", "stopped"),
			"orphan-1":  row("orphan-1", "/b/x", "responding"),
			"orphan-2":  row("orphan-2", "/b/y", "responding"),
			"dead-kid":  row("dead-kid", "/b/z", "stopped"),
			"dead-solo": row("dead-solo", "/c", "stopped"),
		},
		map[string]spawnMeta{
			"live-mgr": {IsWakeTarget: true, Label: "live"},
			"live-kid": {ParentSessionID: "live-mgr"},
			"dead-mgr": {IsWakeTarget: true, Label: "predecessor"},
			"orphan-1": {ParentSessionID: "dead-mgr"},
			"orphan-2": {ParentSessionID: "dead-mgr"},
			// A dead child of a dead parent is NOT an orphan: nothing is waiting.
			"dead-kid": {ParentSessionID: "dead-mgr"},
			// A dead manager with no live children is not a candidate either.
			"gone": {ParentSessionID: "dead-solo"},
		})

	res, err := reg.handle(context.Background(), "agents.orphans", nil)
	if err != nil {
		t.Fatal(err)
	}
	var out struct {
		Candidates []orphanCandidate `json:"candidates"`
		Note       string            `json:"note"`
	}
	if err := json.Unmarshal(res, &out); err != nil {
		t.Fatal(err)
	}
	if len(out.Candidates) != 1 {
		t.Fatalf("want exactly one orphan candidate, got %d: %+v", len(out.Candidates), out.Candidates)
	}
	c := out.Candidates[0]
	if c.SessionID != "dead-mgr" || c.Label != "predecessor" || !c.ConfirmedManager {
		t.Errorf("candidate is %+v — the successor cannot match it against what it was told to take over", c)
	}
	if len(c.Children) != 2 || c.Children[0] != "orphan-1" || c.Children[1] != "orphan-2" {
		t.Errorf("children = %v, want only the LIVE ones, sorted", c.Children)
	}
	if !strings.Contains(out.Note, "do not guess between two candidates") {
		t.Errorf("the note drops the warning that adopting the wrong group re-points another manager's workers: %q", out.Note)
	}

	// "Nothing is orphaned" is a real and common answer and must not read as a
	// failure.
	quiet := fleetReg(t, "http://127.0.0.1:1", map[string]string{"m": row("m", "/a", "input")}, nil)
	res, _ = quiet.handle(context.Background(), "agents.orphans", nil)
	_ = json.Unmarshal(res, &out)
	if len(out.Candidates) != 0 || !strings.Contains(out.Note, "Nothing is orphaned") {
		t.Errorf("an unorphaned fleet reported %d candidate(s): %q", len(out.Candidates), out.Note)
	}
}

// THE REFUSALS ARE THE CONFINEMENT for a verb that acts on OTHER sessions:
// re-pointing live workers at a destination no wake can reach is worse than the
// orphaning it fixes, so each is checked BEFORE anything moves.
func TestReparentRefusesADestinationNoWakeCanReach(t *testing.T) {
	build := func() *registry {
		return fleetReg(t, "http://127.0.0.1:1",
			map[string]string{
				"old":     row("old", "/a", "stopped"),
				"new-mgr": row("new-mgr", "/a", "input"),
				"worker":  row("worker", "/a/x", "responding"),
				"dead":    row("dead", "/b", "stopped"),
				"notmgr":  row("notmgr", "/c", "input"),
			},
			map[string]spawnMeta{
				"new-mgr": {IsWakeTarget: true},
				"worker":  {ParentSessionID: "old"},
			})
	}
	ctx := context.Background()
	for _, c := range []struct{ name, params, wantIn string }{
		{"unknown destination", `{"fromSessionId":"old","toSessionId":"ghost"}`, "no such session"},
		{"ended destination", `{"fromSessionId":"old","toSessionId":"dead"}`, "has ended"},
		{"not a manager", `{"fromSessionId":"old","toSessionId":"notmgr"}`, "is not a manager"},
		{"same session", `{"fromSessionId":"old","toSessionId":"old"}`, "are the same session"},
		{"missing arg", `{"toSessionId":"new-mgr"}`, "requires"},
	} {
		t.Run(c.name, func(t *testing.T) {
			reg := build()
			if _, err := reg.handle(ctx, "agents.reparent", json.RawMessage(c.params)); err == nil {
				t.Fatalf("reparented onto %s — every worker moved this way is silenced", c.name)
			} else if !strings.Contains(err.Error(), c.wantIn) {
				t.Errorf("refusal %q does not contain %q", err, c.wantIn)
			}
			// Nothing moved: a refusal leaves every parent pointer as it was.
			if m, _ := reg.meta.get("worker"); m.ParentSessionID != "old" {
				t.Errorf("a REFUSED reparent still moved the worker to %q", m.ParentSessionID)
			}
		})
	}
}

func TestReparentMovesTheFleetAndIsVisibleImmediately(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := fleetReg(t, srv.URL,
		map[string]string{
			"old":     row("old", "/a", "stopped"),
			"new-mgr": row("new-mgr", "/a", "input"),
			"worker":  row("worker", "/a/x", "responding"),
			"theirs":  row("theirs", "/a/y", "responding"),
		},
		map[string]spawnMeta{
			"new-mgr": {IsWakeTarget: true},
			"worker":  {ParentSessionID: "old"},
			// Not yet registered with the daemon — no live row.
			"pending": {ParentSessionID: "old"},
			// Another manager's worker: must NOT move.
			"theirs": {ParentSessionID: "someone-else"},
		})
	ctx := context.Background()

	res, err := reg.handle(ctx, "agents.reparent", json.RawMessage(`{"fromSessionId":"old","toSessionId":"new-mgr"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out struct {
		Moved   []string `json:"moved"`
		Pending []string `json:"pending"`
		Note    string   `json:"note"`
	}
	_ = json.Unmarshal(res, &out)
	if len(out.Moved) != 1 || out.Moved[0] != "worker" {
		t.Errorf("moved = %v, want the live child only", out.Moved)
	}
	if len(out.Pending) != 1 || out.Pending[0] != "pending" {
		// Dropping a not-yet-registered spawn silently orphans the NEWEST
		// dispatch, which is the one most likely still in flight.
		t.Errorf("pending = %v, want the spawn the daemon has not registered yet", out.Pending)
	}
	if m, _ := reg.meta.get("theirs"); m.ParentSessionID != "someone-else" {
		t.Error("another manager's worker was adopted")
	}

	// Visible on the very next read, not at claudemon's next event: a manager
	// that adopts and immediately lists its fleet must see the move it made.
	snap, _ := reg.store.get("worker")
	var s fleetSession
	_ = json.Unmarshal(snap, &s)
	if s.ParentSessionID != "new-mgr" {
		t.Errorf("the stored row still reports parent %q", s.ParentSessionID)
	}

	// And the moved worker's next progress report now reaches the successor.
	if _, err := reg.handle(ctx, "agents.reportProgress",
		json.RawMessage(`{"callerSessionId":"worker","note":"still going"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(rec.calls("/sessions/new-mgr/message")); n != 1 {
		t.Errorf("the successor received %d wake(s) from its adopted worker", n)
	}
	if n := len(rec.calls("/sessions/old/message")); n != 0 {
		t.Errorf("the retired manager still received %d wake(s)", n)
	}
}

// ── terminals.open ──────────────────────────────────────────────────────────

func TestTerminalsOpenPublishesTheRequestAndRefusesWhenItCannot(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))

	// No bus: REFUSE. Reporting {ok:true} to an agent asking for a terminal the
	// user can SEE, having told nobody, is the silent wrong answer this whole
	// port is about.
	if _, err := reg.handle(context.Background(), "terminals.open", json.RawMessage(`{"cwd":"/p"}`)); err == nil {
		t.Fatal("terminals.open reported success with no way to ask a client for a pane")
	}

	var gotTopic string
	var gotPayload map[string]any
	reg.publish = func(topic string, data json.RawMessage) {
		gotTopic = topic
		_ = json.Unmarshal(data, &gotPayload)
	}
	if _, err := reg.handle(context.Background(), "terminals.open",
		json.RawMessage(`{"cwd":"/p/","command":"npm run dev","label":"dev","parentSessionId":"mgr"}`)); err != nil {
		t.Fatal(err)
	}
	if gotTopic != "facade.openTerminal" {
		t.Fatalf("published %q; the web client subscribes to facade.openTerminal", gotTopic)
	}
	if gotPayload["cwd"] != "/p" {
		t.Errorf("cwd = %v, want the shared normalizeCwd answer", gotPayload["cwd"])
	}
	for k, want := range map[string]any{"command": "npm run dev", "label": "dev", "parentSessionId": "mgr"} {
		if gotPayload[k] != want {
			t.Errorf("payload[%q] = %v, want %v — the client cannot open the pane the agent asked for", k, gotPayload[k], want)
		}
	}
}

// ── sessions.recent ─────────────────────────────────────────────────────────

func TestMergeRecentSessionsMatchesTheDesktopRules(t *testing.T) {
	rows := []daemonSessionRow{
		{SessionID: "agent-synthetic", Cwd: "/x", UpdatedAt: "2026-08-24T12:00:00Z"},
		{SessionID: "", Cwd: "/y"},
		{SessionID: "old", Cwd: "/a", UpdatedAt: "2026-08-20T10:00:00Z", StartedAt: "2026-08-20T09:00:00Z"},
		{SessionID: "new", Cwd: "/b", Provider: "codex", Mode: "input", Transport: "stream", Archived: true, UpdatedAt: "2026-08-24T10:00:00Z"},
		{SessionID: "undated", Cwd: "/c"},
	}
	got := mergeRecentSessions(rows, func(r daemonSessionRow) string { return "name-" + r.SessionID })

	var ids []string
	for _, s := range got {
		ids = append(ids, s.SessionID)
	}
	// "agent-" ids are the desktop's synthetic pre-registration rows, not
	// resumable daemon sessions; an id-less row is not a row.
	want := []string{"new", "old", "undated"}
	if len(ids) != len(want) {
		t.Fatalf("ids = %v, want %v (newest first, synthetic and id-less dropped)", ids, want)
	}
	for i := range want {
		if ids[i] != want[i] {
			t.Fatalf("ids = %v, want %v", ids, want)
		}
	}
	if got[1].Provider != "claude" {
		t.Errorf("a row with an empty provider reported %q; legacy daemon rows serialize claude that way, and the resume path needs a binary to run", got[1].Provider)
	}
	if got[1].Mode != "unknown" || got[1].Transport != "pty" {
		t.Errorf("defaults drifted: mode=%q transport=%q", got[1].Mode, got[1].Transport)
	}
	if !got[0].Archived {
		t.Error("the archived flag was dropped — an archived session is exactly the kind a user resumes")
	}
	if got[0].Name != "name-new" {
		t.Errorf("name = %q; a headless list must still be named wherever this machine knows a name", got[0].Name)
	}
	// Title/Model the headless join cannot answer are EMPTY; cost and tokens
	// are ABSENT (nil), because a zero would render as a measured $0.00.
	if got[0].Title != "" || got[0].Model != "" || got[0].CostUSD != nil ||
		got[0].BilledTokens != nil {
		t.Errorf("headless enrichment invented values: %+v", got[0])
	}
}

// newPathRecordingServer answers one canned body and records the path and query
// it was asked for — the QUERY is the point here (`include_archived`), and the
// shared recorder decodes bodies rather than URLs.
func newPathRecordingServer(t *testing.T, path, query *string, body string) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		*path, *query = r.URL.Path, r.URL.RawQuery
		_, _ = w.Write([]byte(body))
	}))
}

func TestRecentSessionsAsksForArchivedRowsToo(t *testing.T) {
	var gotPath, gotQuery string
	srv := newPathRecordingServer(t, &gotPath, &gotQuery, `[{"session_id":"s1","cwd":"/p"}]`)
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	res, err := reg.handle(context.Background(), "sessions.recent", nil)
	if err != nil {
		t.Fatal(err)
	}
	if gotPath != "/sessions" || !strings.Contains(gotQuery, "include_archived=true") {
		t.Fatalf("asked %s?%s — without include_archived the resume list omits exactly the sessions a user resumes", gotPath, gotQuery)
	}
	var list []recentSession
	if err := json.Unmarshal(res, &list); err != nil || len(list) != 1 {
		t.Fatalf("decoded %v (%v)", list, err)
	}
}

// A daemon that is down yields an EMPTY LIST rather than an error, matching the
// desktop — but it must be a JSON array, not null, or a client that renders
// `.map` over it breaks where it used to show nothing.
func TestRecentSessionsDegradesToAnEmptyArray(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	res, err := reg.handle(context.Background(), "sessions.recent", nil)
	if err != nil {
		t.Fatalf("an unreachable daemon became a call error: %v", err)
	}
	if string(res) != "[]" {
		t.Errorf("answered %s, want []", res)
	}
}

// ── fs.readImage ────────────────────────────────────────────────────────────

// The extension allowlist is the second half of the confinement: without it a
// method named readImage is a general-purpose file reader with a data: URL
// wrapper for anything inside a workspace root.
func TestReadImagePreviewRefusesAnythingNotOnTheAllowlist(t *testing.T) {
	dir := t.TempDir()
	for _, name := range []string{"secret.env", "id_rsa", "notes.md", "archive.tar.gz", "scan.tiff", "noext"} {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte("PLANTED-SECRET"), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := readImagePreview(p); err == nil {
			t.Errorf("readImagePreview served %s — the allowlist is what stops this being a file reader", name)
		}
	}
	// TIFF specifically: on the allowlist for the DESKTOP (which decodes and
	// re-encodes it) and deliberately not here, because no browser renders it
	// and inlining it produces a broken tile where a refusal gives an honest
	// plain chip.
	if _, ok := inlineImageMime["tiff"]; ok {
		t.Error("tiff is on this provider's inline allowlist; no browser renders it")
	}
}

func TestReadImagePreviewServesAnAllowedImageAndHonoursTheCaps(t *testing.T) {
	dir := t.TempDir()
	// A 1x1 PNG, so DecodeConfig has real dimensions to report.
	png := []byte{
		0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a,
		0, 0, 0, 0x0d, 'I', 'H', 'D', 'R',
		0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
		0x1f, 0x15, 0xc4, 0x89,
		0, 0, 0, 0, 'I', 'E', 'N', 'D', 0xae, 0x42, 0x60, 0x82,
	}
	good := filepath.Join(dir, "shot.png")
	if err := os.WriteFile(good, png, 0o600); err != nil {
		t.Fatal(err)
	}
	got, err := readImagePreview(good)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(got.DataURL, "data:image/png;base64,") {
		t.Errorf("dataUrl = %.40s…, want a png data URL the renderer can draw", got.DataURL)
	}
	if got.Width != 1 || got.Height != 1 {
		t.Errorf("dimensions = %dx%d, want 1x1 from the header", got.Width, got.Height)
	}

	// Over the inline cap: refused with a reason that says WHY this provider
	// cannot do what the desktop does, rather than a bare failure.
	big := filepath.Join(dir, "big.png")
	if err := os.WriteFile(big, make([]byte, maxInlineImageBytes+1), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err = readImagePreview(big)
	if err == nil || !strings.Contains(err.Error(), "no image decoder") {
		t.Errorf("an over-cap image was served or refused without a reason: %v", err)
	}

	// A directory named like an image is not a regular file.
	imgDir := filepath.Join(dir, "trap.png.d", "x.png")
	if err := os.MkdirAll(imgDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if _, err := readImagePreview(imgDir); err == nil {
		t.Error("a DIRECTORY was served as an image")
	}
}

// ── brief.append (behaviour; confinement is in briefconfinement_test.go) ────

// THE ADDITIVE GUARANTEE, asserted the way the desktop asserts it: removing the
// inserted line reproduces the input byte for byte. Nothing is rewritten,
// reordered or reflowed, because the user's own edits are authoritative.
func TestAppendToBriefIsStrictlyAdditive(t *testing.T) {
	for _, before := range []string{
		"## Now\n- a\n\n## Direction\n- b\n\n## Recently\n- c\n- d\n",
		"# Project\n\n## Now\n\n- a\n\n\n## Recently\n\n- newest\n- oldest\n",
		"## Now\r\n- windows line\r\n",
		"## Now\n- no trailing newline",
		"## Direction\n- only this section",
	} {
		for _, section := range briefSections {
			next := appendToBrief(before, section, "- INSERTED")
			lines := strings.Split(next, "\n")
			var kept []string
			removed := 0
			for _, l := range lines {
				if l == "- INSERTED" && removed == 0 {
					removed++
					continue
				}
				kept = append(kept, l)
			}
			if removed != 1 {
				t.Fatalf("section %s: inserted %d lines, want exactly 1\nbefore=%q\nafter=%q", section, removed, before, next)
			}
			if got := strings.Join(kept, "\n"); got != before {
				// A missing section is appended, which legitimately adds a
				// heading — that case is checked separately below.
				if strings.Contains(before, "## "+section) {
					t.Errorf("section %s: NOT additive\nbefore=%q\n after=%q", section, before, got)
				}
			}
		}
	}
}

func TestAppendToBriefPlacesTheLineWhereTheSectionIsOrdered(t *testing.T) {
	// Recently is newest-first; every other section appends at the end.
	got := appendToBrief("## Recently\n\n- older\n- oldest\n", "Recently", "- newest")
	if !strings.Contains(got, "## Recently\n\n- newest\n- older") {
		t.Errorf("Recently is a prepend section and the line did not land at the top (below the author's blank line):\n%q", got)
	}
	got = appendToBrief("## Now\n- first\n\n## Direction\n- x\n", "Now", "- second")
	if !strings.Contains(got, "- first\n- second\n\n## Direction") {
		t.Errorf("Now appends at the end of its section, above the author's trailing blank:\n%q", got)
	}
	// A fresh brief gets the doctrine's shape rather than one orphan heading.
	got = appendToBrief("", "Now", "- first line")
	for _, s := range briefSections {
		if !strings.Contains(got, "## "+s) {
			t.Errorf("a fresh brief is missing the %s heading:\n%q", s, got)
		}
	}
	// A missing section is appended without touching a single existing line.
	got = appendToBrief("## Now\n- a\n", "Recently", "- r")
	if !strings.HasPrefix(got, "## Now\n- a\n") || !strings.Contains(got, "\n## Recently\n- r\n") {
		t.Errorf("a missing section was not appended cleanly:\n%q", got)
	}
}

// The one normalization rule that is easy to "simplify" into a bug: interior
// double spaces survive, because the doctrine's own dated-log format uses one.
func TestNormalizeBriefLineKeepsTheDoctrinesDoubleSpace(t *testing.T) {
	got, err := normalizeBriefLine("- 2026-08-24  shipped the thing")
	if err != nil {
		t.Fatal(err)
	}
	if got != "- 2026-08-24  shipped the thing" {
		t.Errorf("got %q — collapsing whitespace makes the tool that writes dated log lines the one thing that cannot write one", got)
	}
	// Newlines ARE flattened: a multi-line insert would break the one-line
	// guarantee everything else rests on.
	got, _ = normalizeBriefLine("wrapped\n   continuation")
	if got != "- wrapped continuation" {
		t.Errorf("got %q, want a single bulleted line", got)
	}
	// Over the cap it REFUSES rather than truncating: the tool is additive-only,
	// so it cannot go back and repair a line it cut.
	if _, err := normalizeBriefLine(strings.Repeat("x", briefLineMax+1)); err == nil {
		t.Error("an over-long line was accepted; a truncated write loses its tail permanently")
	}
	if _, err := normalizeBriefLine("   "); err == nil {
		t.Error("an empty line was accepted")
	}
}

// A typo'd section is REFUSED, not defaulted: silently creating a `## Nwo`
// heading in the user's brief is exactly the damage this tool must not do.
func TestBriefSectionIsRefusedRatherThanDefaulted(t *testing.T) {
	if _, err := parseBriefSection("Nwo"); err == nil {
		t.Fatal("an unknown section was accepted")
	}
	if got, err := parseBriefSection("  recently "); err != nil || got != "Recently" {
		t.Errorf("case-insensitive match failed: %q %v", got, err)
	}
}

func TestBriefAppendWritesUnderTheProjectAndReportsSize(t *testing.T) {
	dir := t.TempDir()
	res, err := briefAppend(briefPathFor(dir), "Recently", "shipped the port")
	if err != nil {
		t.Fatal(err)
	}
	if res["created"] != true {
		t.Errorf("the first append did not report creating the brief: %v", res)
	}
	// The caller named a DIRECTORY; both path components are the provider's.
	wantPath := filepath.Join(dir, ".workspacer", "brief.md")
	if res["path"] != wantPath {
		t.Errorf("wrote %v, want %v", res["path"], wantPath)
	}
	body, err := os.ReadFile(wantPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(body), "- shipped the port") {
		t.Errorf("the line is not in the brief:\n%s", body)
	}
	// The size report is what lets the NEXT append know to run /checkpoint
	// before it hits a read cap, so it must be measured, not guessed.
	if res["entriesInSection"] != 1 {
		t.Errorf("entriesInSection = %v, want 1", res["entriesInSection"])
	}
	if res["bytesInBrief"].(int) != len(body) {
		t.Errorf("bytesInBrief = %v, want %d", res["bytesInBrief"], len(body))
	}

	// N appends land N lines — the lock spans read→compute→write.
	for i := 0; i < 5; i++ {
		if _, err := briefAppend(briefPathFor(dir), "Recently", "line "+string(rune('a'+i))); err != nil {
			t.Fatal(err)
		}
	}
	body, _ = os.ReadFile(wantPath)
	if n := strings.Count(string(body), "\n- "); n != 6 {
		t.Errorf("6 appends produced %d bullets:\n%s", n, body)
	}
}

// Concurrent appends land N lines, never N−1. The lock is the whole reason this
// primitive exists rather than fs.read + fs.write.
func TestBriefAppendIsSerializedAgainstItself(t *testing.T) {
	dir := t.TempDir()
	const n = 8
	done := make(chan error, n)
	for i := 0; i < n; i++ {
		go func(i int) {
			_, err := briefAppend(briefPathFor(dir), "Recently", "concurrent "+strings.Repeat("x", i))
			done <- err
		}(i)
	}
	for i := 0; i < n; i++ {
		if err := <-done; err != nil {
			t.Fatalf("append %d failed: %v", i, err)
		}
	}
	body, err := os.ReadFile(briefPathFor(dir))
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(string(body), "- concurrent"); got != n {
		t.Errorf("%d concurrent appends landed %d lines — the lock did not span read→compute→write:\n%s", n, got, body)
	}
}
