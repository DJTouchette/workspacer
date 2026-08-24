package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"strings"
	"sync"
	"testing"
)

// recorder is a fake claudemon that records the requests it receives and lets a
// test script per-path responses.
type recorder struct {
	mu     sync.Mutex
	hits   []hit
	status map[string]int // path → status code (default 200)
}

type hit struct {
	path string
	body map[string]any
}

func newRecorder() *recorder { return &recorder{status: map[string]int{}} }

func (rec *recorder) server() *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		rec.mu.Lock()
		rec.hits = append(rec.hits, hit{path: r.URL.Path, body: body})
		code := rec.status[r.URL.Path]
		rec.mu.Unlock()
		if code != 0 {
			w.WriteHeader(code)
		}
		// A spawn echoes back a session id; everything else is fine with {ok}.
		if r.URL.Path == "/sessions/spawn" || r.URL.Path == "/sessions/spawn-managed" {
			id, _ := body["session_id"].(string)
			if id == "" {
				id = "generated-id"
			}
			_ = json.NewEncoder(w).Encode(map[string]string{"session_id": id})
			return
		}
		w.Write([]byte(`{"ok":true}`))
	}))
}

func (rec *recorder) calls(path string) []hit {
	rec.mu.Lock()
	defer rec.mu.Unlock()
	var out []hit
	for _, h := range rec.hits {
		if h.path == path {
			out = append(out, h)
		}
	}
	return out
}

// ── claude.answer types into the PTY (matching the app), not /answer ────────

func TestAnswerOptionTypesIntoPTY(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.answer", []byte(`{"sessionId":"s1","option":2}`)); err != nil {
		t.Fatal(err)
	}
	in := rec.calls("/sessions/s1/input")
	if len(in) != 1 || in[0].body["text"] != "2\r" {
		t.Fatalf("expected one input of \"2\\r\", got %+v", in)
	}
	if len(rec.calls("/sessions/s1/answer")) != 0 {
		t.Fatal("answer must not hit the mode-gated /answer endpoint")
	}
}

// An empty answers array unmarshals to a non-nil, zero-length slice. It carries
// no keystrokes, so claude.answer must surface the "requires one of { option,
// text, answers }" error rather than silently returning ok while the agent's
// question picker is left untouched. Covers idx 13.
func TestAnswerEmptyAnswersErrors(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.answer", []byte(`{"sessionId":"s1","answers":[]}`)); err == nil {
		t.Fatal("empty answers array must error, not silently report success")
	}
	// Nothing may be typed into the PTY for an empty answer.
	if in := rec.calls("/sessions/s1/input"); len(in) != 0 {
		t.Fatalf("expected no input to be sent, got %+v", in)
	}
}

// A headless stream-transport session has no PTY, so claude.answer must resolve
// it structurally through POST /answer — never by typing keystrokes into /input
// (which go nowhere and leave the agent hung). Mirrors the desktop's
// transport==='stream' branch. Covers idx 15.
func TestAnswerStreamSessionRoutesToAnswerEndpoint(t *testing.T) {
	var mu sync.Mutex
	var answerBody map[string]any
	var inputCalls, answerCalls int
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodGet && r.URL.Path == "/sessions/s1":
			_ = json.NewEncoder(w).Encode(map[string]any{"session_id": "s1", "transport": "stream"})
			return
		case r.URL.Path == "/sessions/s1/answer":
			mu.Lock()
			answerCalls++
			_ = json.NewDecoder(r.Body).Decode(&answerBody)
			mu.Unlock()
		case r.URL.Path == "/sessions/s1/input":
			mu.Lock()
			inputCalls++
			mu.Unlock()
		}
		w.Write([]byte(`{"ok":true}`))
	}))
	defer srv.Close()

	reg := newRegistry(newClaudemonClient(srv.URL))
	if _, err := reg.handle(context.Background(), "claude.answer", []byte(`{"sessionId":"s1","option":1}`)); err != nil {
		t.Fatal(err)
	}

	mu.Lock()
	defer mu.Unlock()
	if answerCalls != 1 {
		t.Fatalf("stream session must resolve via POST /answer, got %d /answer calls", answerCalls)
	}
	if inputCalls != 0 {
		t.Fatalf("stream session has no PTY; must not type into /input, got %d input calls", inputCalls)
	}
	if opt, _ := answerBody["option"].(float64); opt != 1 {
		t.Errorf("expected option 1 forwarded to /answer, got %v", answerBody["option"])
	}
}

func TestAnswerMultiPart(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.answer", []byte(`{"sessionId":"s1","answers":["yes","blue"]}`)); err != nil {
		t.Fatal(err)
	}
	in := rec.calls("/sessions/s1/input")
	if len(in) != 2 || in[0].body["text"] != "yes\r" || in[1].body["text"] != "blue\r" {
		t.Fatalf("expected two typed answers, got %+v", in)
	}
}

// ── sendMessage surfaces a 409 (ended session) instead of typing blind ──────

func TestSendMessageErrorsOn409(t *testing.T) {
	rec := newRecorder()
	rec.status["/sessions/s1/message"] = http.StatusConflict
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "agents.sendMessage", []byte(`{"sessionId":"s1","text":"hi"}`)); err == nil {
		t.Fatal("a 409 (ended session) must surface as an error, not silently fall back")
	}
	// The old fallback typed the text into the (dead) PTY — that must be gone.
	if n := len(rec.calls("/sessions/s1/input")); n != 0 {
		t.Fatalf("must not type into the PTY on 409, got %d input calls", n)
	}
}

func TestSendMessageNoFallbackOnSuccess(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "agents.sendMessage", []byte(`{"sessionId":"s1","text":"hi"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(rec.calls("/sessions/s1/input")); n != 0 {
		t.Fatalf("happy path must not type into the PTY, got %d input calls", n)
	}
}

// ── terminals.create / gate / resize ────────────────────────────────────────

func TestTerminalsCreateSpawnsShell(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	t.Setenv("SHELL", "/bin/bash")
	reg := newRegistry(newClaudemonClient(srv.URL))

	res, err := reg.handle(context.Background(), "terminals.create", []byte(`{"cwd":"/tmp"}`))
	if err != nil {
		t.Fatal(err)
	}
	spawn := rec.calls("/sessions/spawn")
	if len(spawn) != 1 {
		t.Fatalf("expected one spawn, got %d", len(spawn))
	}
	argv, _ := spawn[0].body["argv"].([]any)
	if len(argv) != 1 || argv[0] != "/bin/bash" {
		t.Errorf("expected argv [/bin/bash], got %v", argv)
	}
	if _, pinned := spawn[0].body["session_id"]; pinned {
		t.Error("a shell should not pin a session_id")
	}
	var out struct {
		SessionID string `json:"sessionId"`
	}
	if json.Unmarshal(res, &out); out.SessionID == "" {
		t.Error("expected a sessionId back")
	}
}

func TestGateForwards(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.gate", []byte(`{"sessionId":"s1","on":true}`)); err != nil {
		t.Fatal(err)
	}
	g := rec.calls("/sessions/s1/gate")
	if len(g) != 1 || g[0].body["on"] != true {
		t.Fatalf("expected gate on=true, got %+v", g)
	}
}

func TestTerminalResizeForwards(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "sessions.terminalResize", []byte(`{"sessionId":"s1","cols":100,"rows":40}`)); err != nil {
		t.Fatal(err)
	}
	r := rec.calls("/sessions/s1/resize")
	if len(r) != 1 || r[0].body["cols"] != float64(100) || r[0].body["rows"] != float64(40) {
		t.Fatalf("expected resize 100x40, got %+v", r)
	}
}

// ── profiles CRUD round-trips through the same file the app reads ────────────

func TestProfilesCRUD(t *testing.T) {
	dir := tempConfigHome(t)

	added, err := addProfile("Work", "~/work-cfg", []string{"--foo"}, []string{"mcp-1"})
	if err != nil {
		t.Fatal(err)
	}
	// NOT default: loadProfiles materializes the "Default" row first, exactly as
	// claudeProfiles.ts's constructor does, so the added profile is the second.
	// The brain used to prepend that row synthetically without writing it, which
	// made this same call mint isDefault:true here and isDefault:false there.
	if added.ID == "" || added.IsDefault {
		t.Fatalf("an added profile is not the default once Default exists, got %+v", added)
	}

	// Persisted to claude-profiles.json in the shape the app expects.
	raw, err := os.ReadFile(filepath.Join(dir, "workspacer", "claude-profiles.json"))
	if err != nil {
		t.Fatal(err)
	}
	var file struct {
		Profiles []profile `json:"profiles"`
	}
	if err := json.Unmarshal(raw, &file); err != nil || len(file.Profiles) != 2 ||
		file.Profiles[0].ID != "default" || !file.Profiles[0].IsDefault {
		t.Fatalf("expected [Default, Work] persisted, got %s (err %v)", raw, err)
	}

	// Update name; isDefault stays.
	name := "Work2"
	updated, err := updateProfile(added.ID, profileUpdate{Name: &name})
	if err != nil || updated.Name != "Work2" {
		t.Fatalf("update failed: %+v err %v", updated, err)
	}

	// A second profile is not default.
	second, err := addProfile("Play", "", nil, nil)
	if err != nil || second.IsDefault {
		t.Fatalf("second profile should not be default: %+v err %v", second, err)
	}

	// Removing a non-default leaves the Default in place.
	if err := removeProfile(added.ID); err != nil {
		t.Fatal(err)
	}
	left := readProfilesFile()
	if len(left) != 2 || left[0].ID != "default" || left[1].ID != second.ID {
		t.Fatalf("after removing Work, [Default, Play] should remain: %+v", left)
	}

	// And removing the DEFAULT is refused on both providers (claudeProfiles.ts:
	// `if (id === 'default') return`), so a caller cannot leave the store with no
	// default at all.
	if err := removeProfile("default"); err != nil {
		t.Fatal(err)
	}
	if left := readProfilesFile(); len(left) != 2 {
		t.Fatalf("removing the Default profile must be a no-op: %+v", left)
	}
}

// ── host fs ops ─────────────────────────────────────────────────────────────

func TestFsReadWriteRoundTrip(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "note.txt")
	// The temp dir has to be a live agent cwd for the write to be allowed at all
	// — fs.* is confined to agent cwds + the config dir (see fsguard_test.go).
	reg := registryWithCwd(t, dir)

	if _, err := reg.handle(context.Background(), "fs.write",
		json.RawMessage(`{"path":`+jsonStr(p)+`,"contents":"hello"}`)); err != nil {
		t.Fatal(err)
	}
	res, err := reg.handle(context.Background(), "fs.read", json.RawMessage(`{"path":`+jsonStr(p)+`}`))
	if err != nil {
		t.Fatal(err)
	}
	var got readFileResult
	if json.Unmarshal(res, &got); got.Contents != "hello" || got.Size != 5 {
		t.Fatalf("read back %+v, want contents hello size 5", got)
	}
}

func TestFsListDirReturnsDirsOnly(t *testing.T) {
	dir := t.TempDir()
	_ = os.Mkdir(filepath.Join(dir, "visible"), 0o755)
	_ = os.Mkdir(filepath.Join(dir, ".hidden"), 0o755)
	_ = os.WriteFile(filepath.Join(dir, "file.txt"), []byte("x"), 0o644)

	res, err := listHostDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Dirs) != 1 || res.Dirs[0] != "visible" {
		t.Fatalf("expected only [visible], got %v", res.Dirs)
	}
	if res.Home == "" || res.Parent == "" {
		t.Fatalf("expected home+parent populated, got %+v", res)
	}
}

func jsonStr(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}

// ── agents.spawn param-surface drift guard ──────────────────────────────────

// spawnParamsDeclined lists desktop agents.spawn params the brain deliberately
// does NOT mirror, with the reason. A param here must still exist on the
// desktop side (prune the entry when it's removed there); a desktop param that
// is neither in spawnParams' JSON tags nor here fails the drift guard below —
// mirror it or decline it explicitly.
var spawnParamsDeclined = map[string]string{
	"mcpFacade": "the workspacer MCP facade server runs inside the desktop app; headless there is no facade URL to wire",
	// `manager` IS mirrored (see spawnParams.Manager) — only its full-access
	// COMPANION is declined, and for the same reason as mcpFacade.
	"fleetFullAccess": "inert on the wire even on the desktop: nothing reads the spawn option — the manager's actual bypass is resolved from CONFIG at mint time (fullAccessGrants.managerFullAccessFromConfig), deliberately not from this flag, so a respawn's frozen copy cannot resurrect a revoked grant. The only thing it is a record OF is the role-tagged grant on a minted session facade token, which headless cannot mint (see mcpFacade/toolScope). Mirroring it would add a param the brain accepts and provably never honors; the bypass a bus caller can legitimately obtain here already rides the hub-verified `yoloGranted` stamp instead",
	"mcpItemIds":      "per-spawn Library MCP servers need buildSessionMcpConfig (a desktop-owned session-scoped --mcp-config writer)",
	"toolScope":       "the facade tool tier rides the facade (desktop-only, see mcpFacade) — minting/revoking the per-session token is desktop-owned (remoteTokens.ts)",
	"pluginTools":     "plugin tool grants are recorded on the session facade token, which headless cannot mint (see toolScope)",
	"worktree":        "ship-task worktree isolation is created by the desktop (worktreeService.createWorktree) before the spawn; the headless brain has no equivalent worktree pool, so a worktree spawn it answers just runs in cwd",
	"resultSchema":    "the structured-result contract has TWO desktop-owned halves and neither exists headless: the schema is compiled into the worker's spawn prompt (claudeSpawn's --append-system-prompt / managedSpawn's first-turn instructions) and it is READ BACK by supervisorNudge's worker-finished wake, which is a desktop session-store facility — the brain has no session store holding the schema and no wake to deliver the validated object on, so a brain-answered spawn would take the param and silently never honor it",
}

// spawnParamsAhead is the mirror escape hatch: brain spawnParams the DESKTOP
// does not take yet, with the reason the brain leads. The symmetric rule to
// spawnParamsDeclined — an entry here must NOT exist on the desktop side (the
// moment hubCapabilities.ts grows the param, this guard fails until the entry
// is pruned), and must be a real brain JSON tag, so the exception can neither
// linger past the desktop catching up nor outlive the field it excuses.
var spawnParamsAhead = map[string]string{}

// desktopSpawnParamRe pulls the field names out of the agents.spawn params type
// literal in hubCapabilities.ts (`provider?: AgentProvider;` → provider).
var desktopSpawnParamRe = regexp.MustCompile(`(?m)^\s*(\w+)\?:`)

// TestSpawnParamSurfaceMatchesDesktop cross-checks the desktop's agents.spawn
// param list (parsed from hubCapabilities.ts) against the brain's spawnParams
// JSON tags, so a param added on the desktop side fails here until the brain
// mirrors it (or documents why not in spawnParamsDeclined). The behavioural
// counterpart of capspec_guard_test's method-name cross-check. Skips (not
// fails) when the TS source isn't reachable (e.g. a hub-only checkout).
func TestSpawnParamSurfaceMatchesDesktop(t *testing.T) {
	// A missing twin FAILS; only an absent checkout skips (mustReadRepoFile).
	data := mustReadRepoFile(t, "apps", "desktop", "src", "main", "services", "hubCapabilities.ts")
	text := string(data)
	// Isolate the agents.spawn registration's destructure type literal.
	start := strings.Index(text, "registerCapability('agents.spawn'")
	if start < 0 {
		t.Fatal("hubCapabilities.ts no longer registers 'agents.spawn' — update this guard")
	}
	text = text[start:]
	open := strings.Index(text, "} = (params ?? {}) as {")
	end := strings.Index(text[open+1:], "};")
	if open < 0 || end < 0 {
		t.Fatal("could not find the agents.spawn params type literal — the destructuring syntax changed; update this guard")
	}
	block := text[open : open+1+end]

	desktop := map[string]bool{}
	for _, m := range desktopSpawnParamRe.FindAllStringSubmatch(block, -1) {
		desktop[m[1]] = true
	}
	if len(desktop) < 5 {
		t.Fatalf("parsed implausibly few desktop spawn params (%v) — the regex stopped matching", desktop)
	}

	brain := map[string]bool{}
	tp := reflect.TypeOf(spawnParams{})
	for i := 0; i < tp.NumField(); i++ {
		if tag := strings.Split(tp.Field(i).Tag.Get("json"), ",")[0]; tag != "" {
			brain[tag] = true
		}
	}

	for param := range desktop {
		if !brain[param] && spawnParamsDeclined[param] == "" {
			t.Errorf("desktop agents.spawn takes %q but the brain's spawnParams doesn't — mirror it or add it to spawnParamsDeclined with a reason", param)
		}
	}
	for param := range brain {
		if !desktop[param] && spawnParamsAhead[param] == "" {
			t.Errorf("brain spawnParams has %q but the desktop's agents.spawn doesn't — the surfaces must stay identical (or record why the brain leads in spawnParamsAhead)", param)
		}
	}
	for param := range spawnParamsAhead {
		if desktop[param] {
			t.Errorf("spawnParamsAhead lists %q but the desktop now takes it — the surfaces converged; prune the entry", param)
		}
		if !brain[param] {
			t.Errorf("spawnParamsAhead lists %q but the brain's spawnParams doesn't have it — the exception outlived the field", param)
		}
	}
	for param := range spawnParamsDeclined {
		if !desktop[param] {
			t.Errorf("spawnParamsDeclined lists %q but the desktop no longer takes it — prune the entry", param)
		}
		if brain[param] {
			t.Errorf("%q is both mirrored in spawnParams and declined in spawnParamsDeclined — drop one", param)
		}
	}
}

// ── snapshot field-shape drift guard ─────────────────────────────────────────

// snapshotFieldsRequired lists the desktop-snapshot (camelCase) fields the bus
// clients read off a session row — mobile.html directly, the web renderer via
// webBackend — that compatSnapshot must therefore emit (or pass through) on
// every brain-served row. Adding a snapshot read to mobile.html without
// teaching the overlay fails here; pruning a field there should prune it here.
var snapshotFieldsRequired = []string{
	"sessionId",
	"status",
	"ambientState",
	"lastActivity",
	"cwd",
	"transport",
	"provider",
	"usage",
	"pendingApproval",
	"pendingQuestions",
	"statusLine",
	"totalToolCalls",
}

// snapshotFieldsDeclined lists desktop-snapshot fields the clients read that
// the brain deliberately does NOT provide, with the reason. Entries must still
// be read by mobile.html (prune when the client stops using them).
var snapshotFieldsDeclined = map[string]string{
	"conversation": "turn-by-turn transcript lives in claudemon's /conversation endpoint, not the session row; folding it into every snapshot/publish would ship whole transcripts per state tick — mobile fetches it on demand via sessions.conversation instead",
	"liveCwd":      "statusline-derived live cwd is a desktop enrichment; clients fall back to cwd (mobile agentPath: liveCwd || cwd). NOTE: mobile's card TITLE is deliberately cwd-only — liveCwd follows an agent into a worktree, and titling from it renamed a running card mid-dispatch",
}

// TestCompatSnapshotCoversMobileFields cross-checks the field names the mobile
// client reads against the compat overlay — the wire-shape counterpart of the
// spawn-param guard above. FAILS (never skips) when mobile.html has moved.
func TestCompatSnapshotCoversMobileFields(t *testing.T) {
	// A missing twin FAILS; only an absent checkout skips (mustReadRepoFile).
	data := mustReadRepoFile(t, "services", "hub", "cmd", "hub", "mobile.html")
	mobile := string(data)

	// A representative claudemon row exercising every mapped branch.
	row := json.RawMessage(`{
		"session_id":"s1","mode":"responding","cwd":"/tmp","provider":"claude",
		"transport":"stream","archived":false,"updated_at":"2026-07-10T12:00:00Z",
		"usage":{"model":"m","context_tokens":1,"context_limit":2,"cost_usd":0.1},
		"tool_calls":7,
		"status_line":{"model_display":"Opus","context_used_pct":12.5,"context_window_size":200000,
			"total_input_tokens":100,"total_output_tokens":50,"cost_usd":0.2,
			"five_hour_pct":10,"five_hour_resets_at":1234,"seven_day_pct":20,"seven_day_resets_at":5678,
			"monthly_pct":30,"monthly_resets_at":9012,"rate_limit_warning":"careful",
			"received_at":"2026-07-10T12:00:00Z"},
		"pending":null}`)
	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatal(err)
	}

	for _, field := range snapshotFieldsRequired {
		if !strings.Contains(mobile, field) {
			t.Errorf("snapshotFieldsRequired lists %q but mobile.html no longer references it — prune the entry", field)
		}
		if _, ok := m[field]; !ok {
			t.Errorf("mobile reads snapshot field %q but compatSnapshot doesn't emit it — map it or decline it with a reason", field)
		}
	}
	for field, reason := range snapshotFieldsDeclined {
		if reason == "" {
			t.Errorf("snapshotFieldsDeclined[%q] needs a reason", field)
		}
		if !strings.Contains(mobile, field) {
			t.Errorf("snapshotFieldsDeclined lists %q but mobile.html no longer references it — prune the entry", field)
		}
	}
	// usage sub-shape: mobile reads u.contextTokens / contextLimit / costUSD /
	// model (ctxPct, fleetCard, shortModel).
	u, _ := m["usage"].(map[string]any)
	for _, k := range []string{"model", "contextTokens", "contextLimit", "costUSD"} {
		if _, ok := u[k]; !ok {
			t.Errorf("usage overlay missing %q (mobile ctxPct/fleetCard read it)", k)
		}
	}
	// statusLine sub-shape: mobile reads these camelCase off s.statusLine (stats,
	// progressFingerprint, statusLineReceivedAt) — claudemon's row carries them
	// snake_case under status_line, so compatSnapshot must rename every one.
	sl, _ := m["statusLine"].(map[string]any)
	for _, k := range []string{
		"modelDisplay", "contextUsedPct", "contextWindowSize", "totalInputTokens",
		"totalOutputTokens", "costUSD", "fiveHourPct", "fiveHourResetsAt",
		"sevenDayPct", "sevenDayResetsAt", "monthlyPct", "monthlyResetsAt",
		"rateLimitWarning", "receivedAt",
	} {
		if _, ok := sl[k]; !ok {
			t.Errorf("statusLine overlay missing %q (mobile stats/progressFingerprint read it)", k)
		}
	}
}

// nestingFieldsRequired are the spawn-metadata fields the /m client renders the
// FLEET HIERARCHY from: the manager card, its MANAGER chip, and the crew rail
// of workers nested beneath it (mobile.html fleetRoster / isManager / agentName).
// They ride enrichSnapshot, not compatSnapshot, so they need their own guard —
// without one, a brain that stopped enriching would flatten the phone's fleet
// back into an undifferentiated list with no test saying so.
var nestingFieldsRequired = []string{
	"parentSessionId", // who dispatched this worker → the crew it renders in
	"isSupervisor",    // manager/supervisor → the MANAGER chip and the group anchor
	"label",           // the STATIC card title: the task the worker was dispatched with
}

func TestEnrichSnapshotCoversMobileNestingFields(t *testing.T) {
	data := mustReadRepoFile(t, "services", "hub", "cmd", "hub", "mobile.html")
	mobile := string(data)

	meta := newMetaStore()
	meta.set("w1", spawnMeta{Label: "proj: a task", ParentSessionID: "mgr", IsSupervisor: true})
	var m map[string]any
	enriched := enrichSnapshot(json.RawMessage(`{"session_id":"w1","cwd":"/tmp"}`), meta)
	if err := json.Unmarshal(enriched, &m); err != nil {
		t.Fatal(err)
	}
	for _, field := range nestingFieldsRequired {
		if !strings.Contains(mobile, field) {
			t.Errorf("nestingFieldsRequired lists %q but mobile.html no longer reads it — prune the entry", field)
		}
		if _, ok := m[field]; !ok {
			t.Errorf("mobile nests the fleet on %q but enrichSnapshot doesn't emit it", field)
		}
	}
}
