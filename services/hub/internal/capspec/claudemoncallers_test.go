package capspec

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// THE CALLER HALF OF THE claudemon HTTP SEAM.
//
// httproutes.go already scans claudemon's two axum routers and refuses a route
// nobody classified. That is the SERVER end. Nothing anywhere asked the other
// question — does every path a CLIENT builds still resolve to a route the daemon
// serves — and the answer, for six weeks, was no: commit 37320188 deleted the
// /git family and apps/tui kept calling /git/status, /git/diff, /git/stage,
// /git/unstage, /git/commit and /git/push. The suite was green the whole time.
// It could not have been anything else: apps/tui's mock_server answered the
// configured status for ANY path, so ~20 route assertions pinned the caller's
// own URL against itself, and four review-pane tests asserted in-process dock
// state while six live 404s went by underneath.
//
// The bus seam — the other string-keyed cross-process surface — has had this
// guard for a while (cmd/brain/headless_completeness_test.go) and a full sweep
// of it came back clean. One seam had a contract and stayed clean; the other had
// none and rotted. That is the whole argument for this file.
//
// WHERE THIS LIVES AND WHY. This package already scans Go, Rust and TypeScript
// registration sites from one place — "a boundary that stops at a language
// boundary is not a boundary" (httproutes.go) — and it already owns the served
// route table. The callers are Rust (apps/tui, claudemon's own watch TUI and
// wrapper/provider glue), TypeScript (apps/desktop's main process) and Go (the
// brain, the status probe). No single language's test runner can see all three,
// so the sweep sits with the registry it compares against, and the fixture
// contracts/claudemon-routes.json is what carries the served table across to the
// Rust side (apps/tui's mock_server reads it too).
//
// FOUR GUARDS, and the last two are the ones that answer "what did the
// enumeration miss":
//
//   - TestClaudemonRouteFixtureMatchesTheServedRegistry — the fixture is the
//     same 37 routes this package's own scan finds. Two independent derivations
//     of one table, so neither can drift alone.
//   - TestEveryClaudemonCallerPathIsServed — the sweep itself.
//   - TestEveryClaudemonCallerFileIsEnumerated — a file anywhere in the repo
//     that reaches for a claudemon base and is not on the caller list fails.
//     headless_completeness_test.go enumerates exactly three client files and
//     does not see wks-tui; an enumeration that silently misses a client is the
//     same blind spot one level up, so this list is held to a repo-wide scan.
//   - TestEveryClaudemonRouteHasACallerOrADeclaredReason — the mirror image. A
//     route nothing in the repo calls is either serving an external caller or is
//     dead weight, and which one it is has to be written down. GET /usage/report
//     is the row that made this test worth having: it had served the widest read
//     on the loopback plane since 0.160.0 with no caller in any commit, and it
//     has one now — keep-warm reads Codex's 5h window from it rather than from
//     live status lines, which is a question only this route can answer with no
//     Codex session running. The row is gone because the guard also fails a
//     STALE declaration, which is how the closing of an orphan gets noticed.

// claudemonRoutesFixture is the served table, derived from the routers by
// services/claudemon/src/daemon/routes_contract.rs.
var claudemonRoutesFixture = []string{"contracts", "claudemon-routes.json"}

type fixtureRoute struct {
	Server  string `json:"server"`
	Pattern string `json:"pattern"`
	Method  string `json:"method"`
}

// callerScan is one declared claudemon client, and the shape its URLs take.
//
// The regexes are per-file on purpose: these callers spell a URL five different
// ways (a Rust bare path literal, a Rust format! with a positional base, a
// TypeScript template hole, a Go `+` chain, a curl command line baked into a
// settings.json), and one loose pattern over all of them would either miss most
// of it or drown in filesystem paths. `floor` is what keeps a regex that stops
// matching from reporting a clean sweep over nothing.
type callerScan struct {
	// file is repo-relative.
	file string
	// server is which of the two routers this caller talks to.
	server string
	// what the caller is, in one line, for the failure message.
	what string
	// res each capture group 1 = the path (or the part of the URL after the
	// base).
	res []*regexp.Regexp
	// suffix is appended to every path this entry finds. Used where the base and
	// the tail are composed in different files.
	suffix string
	// floor is the minimum number of paths the scan must find here.
	floor int
	// cutRustTests truncates the source at #[cfg(test)] — a path inside a test
	// module is a fixture, not a call.
	cutRustTests bool
}

var (
	// A Rust bare path literal. In apps/tui/src/claudemon.rs every "/…" literal
	// outside the test module IS a claudemon path — there is no other kind — so
	// the most sensitive possible rule is also the correct one here.
	rustPathLiteralRe = regexp.MustCompile(`"(/[^"\n]*)"`)
	// format!("{}/sessions/{}/transcript", self.api_url, id) — the base is the
	// leading positional hole.
	rustFormatBaseRe = regexp.MustCompile(`"\{\}(/[^"\n]*)"`)
	// format!("{api_base}/mcp/ask/{session_id}") — the named-hole spelling the
	// provider adapters use when they hand an agent a callback URL.
	rustApiBaseRe = regexp.MustCompile(`\{api_base\}(/[^"\n\\]*)`)
	// The curl command lines claudemon init writes into ~/.claude/settings.json.
	// Claude Code runs these; a renamed hook route breaks them silently.
	rustHookCurlRe = regexp.MustCompile(`127\.0\.0\.1:\{hook_port\}(/[^\s"\\]*)`)
	// ws://127.0.0.1:7891/wrapper — the wrapper's default daemon URL.
	rustWrapperBaseRe = regexp.MustCompile(`"ws://127\.0\.0\.1:7891(/[^"\n]*)"`)

	// `${CLAUDEMON_API_URL}/sessions/${id}/model` in a template literal.
	tsClaudemonBaseRe = regexp.MustCompile("\\$\\{CLAUDEMON_API_URL\\}([^`'\"]*)")
	// `http://127.0.0.1:${API_PORT}/health` — claudemonDaemon.ts's own probe,
	// which builds the base rather than importing it.
	tsApiPortRe = regexp.MustCompile("\\$\\{API_PORT\\}([^`'\"]*)")
	// this.postJSON(`/sessions/${id}/approve`, …) — claudemonSessionClient's one
	// indirection, where the path reaches the base through a helper.
	tsPostJSONRe = regexp.MustCompile("postJSON\\(\\s*`([^`]*)`")

	// A Go path literal, read after goConcatChains has folded `"/sessions/" + id
	// + "/input"` into a single literal.
	goPathLiteralRe = regexp.MustCompile(`"(/[^"\n]*)"`)
	// probeClaudemon's `base+"/health"` / `base+"/sessions"`.
	goBasePlusRe = regexp.MustCompile(`\bbase\s*\+\s*"(/[^"\n]*)"`)
	// The documented default in cmd/hub's --claudemon-events flag, which is the
	// URL the SSE bridge is pointed at in every shipped configuration.
	goLoopbackURLRe = regexp.MustCompile(`127\.0\.0\.1:7891(/[^\s")\n]*)`)
)

// claudemonCallers is THE ENUMERATION. Every process in this repo that speaks
// HTTP to claudemon appears here, and TestEveryClaudemonCallerFileIsEnumerated
// holds the list to a repo-wide scan so it cannot quietly fall behind.
var claudemonCallers = []callerScan{
	// ---- Rust: the terminal client (the one that rotted) -----------------
	{
		file: "apps/tui/src/claudemon.rs", server: "claudemon-api",
		what:         "wks-tui's whole claudemon client: list, conversation, spawn, approve/answer/message/signal, the live-control switches and the SSE feeds",
		res:          []*regexp.Regexp{rustPathLiteralRe},
		floor:        16,
		cutRustTests: true,
	},

	// ---- Rust: claudemon's own surfaces ----------------------------------
	{
		file: "services/claudemon/src/tui/app.rs", server: "claudemon-api",
		what:         "`claudemon watch` — the daemon's own attached TUI, which talks to the API over loopback like any other client",
		res:          []*regexp.Regexp{rustFormatBaseRe},
		floor:        6,
		cutRustTests: true,
	},
	{
		file: "services/claudemon/src/cli.rs", server: "claudemon-api",
		what:         "the `claudemon wrap` default daemon URL. wrapper/mod.rs appends `/{session_id}` to it, which is why this row carries a suffix",
		res:          []*regexp.Regexp{rustWrapperBaseRe},
		suffix:       "/:id",
		floor:        1,
		cutRustTests: true,
	},
	{
		file: "services/claudemon/src/daemon/init.rs", server: "claudemon-hook",
		what:         "the curl command lines written into ~/.claude/settings.json. CLAUDE CODE is the caller here, not us — a renamed hook route leaves a shipped settings file POSTing into a 404 with no error path at all",
		res:          []*regexp.Regexp{rustHookCurlRe},
		floor:        2,
		cutRustTests: true,
	},
	{
		file: "services/claudemon/src/providers/codex.rs", server: "claudemon-api",
		what:         "the AskUserQuestion callback URL handed to the codex CLI as an MCP server",
		res:          []*regexp.Regexp{rustApiBaseRe},
		floor:        1,
		cutRustTests: true,
	},
	{
		file: "services/claudemon/src/providers/copilot.rs", server: "claudemon-api",
		what:         "the AskUserQuestion callback URL handed to the copilot CLI",
		res:          []*regexp.Regexp{rustApiBaseRe},
		floor:        1,
		cutRustTests: true,
	},
	{
		file: "services/claudemon/src/providers/opencode.rs", server: "claudemon-api",
		what:         "the AskUserQuestion callback URL handed to opencode",
		res:          []*regexp.Regexp{rustApiBaseRe},
		floor:        1,
		cutRustTests: true,
	},
	{
		file: "services/claudemon/src/providers/pi.rs", server: "claudemon-api",
		what:         "the AskUserQuestion callback URL baked into the pi extension source",
		res:          []*regexp.Regexp{rustApiBaseRe},
		floor:        1,
		cutRustTests: true,
	},

	// ---- TypeScript: the desktop main process ----------------------------
	{
		file: "apps/desktop/src/main/services/claudemonSessionClient.ts", server: "claudemon-api",
		what:  "the desktop's session client — spawn, snapshot, PTY stream, message, live model/permission-mode switch, transcript, conversation, handoff",
		res:   []*regexp.Regexp{tsClaudemonBaseRe, tsPostJSONRe},
		floor: 14,
	},
	{
		file: "apps/desktop/src/main/services/claudemonEventBridge.ts", server: "claudemon-api",
		what: "the /events SSE bridge onto the desktop session store", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},
	{
		file: "apps/desktop/src/main/services/claudemonHookBridge.ts", server: "claudemon-api",
		what: "the /hooks/stream SSE bridge", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},
	{
		file: "apps/desktop/src/main/services/claudemonStatusLineBridge.ts", server: "claudemon-api",
		what: "the /statusline/stream SSE bridge (cost, context and rate-limit frames)", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},
	{
		file: "apps/desktop/src/main/services/claudemonConversationBridge.ts", server: "claudemon-api",
		what: "the /conversation/stream delta feed", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},
	{
		file: "apps/desktop/src/main/services/claudemonDaemon.ts", server: "claudemon-api",
		what: "the daemon supervisor's own adopt/health probe", res: []*regexp.Regexp{tsApiPortRe}, floor: 2,
	},
	{
		file: "apps/desktop/src/main/services/recentSessions.ts", server: "claudemon-api",
		what: "the RECENT / History listing", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 2,
	},
	{
		file: "apps/desktop/src/main/services/keepWarmService.ts", server: "claudemon-api",
		what: "the 5h-window warmer: reads /usage for Claude's default login, /usage/report for Codex's on-disk window, and posts /heartbeat", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 3,
	},
	{
		file: "apps/desktop/src/main/services/directCompletion.ts", server: "claudemon-api",
		what: "agent auto-titling through POST /oneshot", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},
	{
		file: "apps/desktop/src/main/services/claudeSessionStore.ts", server: "claudemon-api",
		what: "the session store's conversation resync", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},
	{
		file: "apps/desktop/src/main/services/terminalShare.ts", server: "claudemon-api",
		what: "the remote terminal mirror: the PTY byte stream plus the snapshot beside it", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 2,
	},
	{
		file: "apps/desktop/src/main/services/hubDaemon.ts", server: "claudemon-api",
		what: "the --claudemon-events URL the desktop hands the supervised hub", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},
	{
		file: "apps/desktop/src/main/ipc.ts", server: "claudemon-api",
		what: "the heartbeats IPC handler", res: []*regexp.Regexp{tsClaudemonBaseRe}, floor: 1,
	},

	// ---- Go: the headless side -------------------------------------------
	{
		file: "services/hub/cmd/brain/claudemon.go", server: "claudemon-api",
		what:  "the brain's claudemon client — the provider behind every bus/web/mobile/MCP caller on a headless node",
		res:   []*regexp.Regexp{goPathLiteralRe},
		floor: 18,
	},
	{
		file: "services/hub/cmd/workspacer/status.go", server: "claudemon-api",
		what:  "`workspacer status`'s claudemon probe",
		res:   []*regexp.Regexp{goBasePlusRe},
		floor: 2,
	},
	{
		file: "services/hub/cmd/hub/main.go", server: "claudemon-api",
		what:  "the --claudemon-events SSE URL the hub bridges onto the bus",
		res:   []*regexp.Regexp{goLoopbackURLRe},
		floor: 1,
	},
}

// claudemonRouteCallers is the mirror-image declaration: a route that nothing in
// this repo calls, and why that is acceptable. Same doctrine as headlessGaps —
// the guard does not demand that every orphan be closed, it demands that every
// orphan be KNOWN.
var claudemonRouteCallers = map[string]string{
	// claudemon-api
	"claudemon-api /sessions/:id/decide": "the DECISION-record half of the approval surface. The desktop and the TUI both answer a parked prompt through /approve; /decide is reachable for a caller that has a decision record rather than a yes/no, and nothing in the repo is that caller yet. Serving it costs nothing and it shares post_approve's validation",
	"claudemon-api /sessions/:id/output": "the raw PTY scrollback as a one-shot GET. Every in-repo client takes the same bytes from /sessions/:id/stream instead, because they all want the live feed and its snapshot-replay first frame. Kept for a caller that wants a single read without holding a stream",

	// claudemon-hook — the callers are Claude Code's own hook shellouts, not us.
	"claudemon-hook /hook/:kind": "the per-kind ingress. claudemon init writes the generic `/hook` form into ~/.claude/settings.json, so the sub-routed spelling is for a caller that wants the kind in the URL rather than the body — hook.rs 404s an unknown :kind on purpose, which is the only reason this route is safe to leave uncalled",
	"claudemon-hook /health":     "liveness on the ingress port. Nothing probes it today; the API port's /health is what every supervisor waits on",
}

// claudemonNonCallers are files the discovery scan flags that build no route.
// Each needs a reason, for the same reason a headlessGaps entry does: "it is
// fine" is not checkable, and an exemption with no sentence behind it is where
// the next dead caller will hide.
var claudemonNonCallers = map[string]string{
	"apps/desktop/src/main/lib/daemonUtils.ts":                 "declares the PORTS registry and the shared health-poll helper; it composes no claudemon path of its own",
	"apps/desktop/src/main/services/mcpFacadeDaemon.ts":        "supervises the MCP facade on :7897 and probes ITS /health, not claudemon's",
	"apps/desktop/src/renderer/src/lib/changelog.generated.ts": "generated release-note prose that quotes endpoint names; it makes no request",
	"apps/tui/src/main.rs":                                     "parses the --claudemon base URL and hands it to claudemon.rs; it builds no path",
	"services/claudemon/src/daemon/api.rs":                     "the API router itself — the served side of this contract, scanned by httproutes_test.go",
	"services/claudemon/src/daemon/hook.rs":                    "the hook router itself",
	"services/claudemon/src/daemon/wrapper_ws.rs":              "the /wrapper/:id upgrade handler — the served side; the 127.0.0.1:7891 spellings in it are a doc comment and origin-guard test inputs",
	"services/claudemon/src/tui/preview.rs":                    "a screenshot harness that constructs the watch TUI against a loopback base and renders it without making a request",
	"services/claudemon/src/daemon/mod.rs":                     "binds the two listeners and records API_BASE; the callback URLs are composed in the provider adapters, which are enumerated above",
	"services/claudemon/src/wrapper/mod.rs":                    "appends /{session_id} to the base cli.rs supplies; that composition is declared on the cli.rs row's suffix",
	"services/hub/cmd/brain/main.go":                           "parses the --claudemon flag and constructs the client; every path lives in claudemon.go",
	"services/hub/cmd/hub/brain.go":                            "passes the claudemon base URL through to the supervised brain as an argv element",
	"services/hub/cmd/workspacer/serve.go":                     "holds the default claudemon port for the launcher's spawn plan; it makes no request",
	"services/hub/internal/capspec/httproutes.go":              "the route registry this file compares against",
	"services/hub/internal/claudemon/bridge.go":                "consumes whatever SSE URL it is handed; the URL is composed by cmd/hub/main.go, which is enumerated above",
}

// claudemonBaseMarkers are the spellings a file uses to reach for a claudemon
// listener. A file containing one of these is claimed by the discovery test:
// it is a declared caller, or a declared non-caller with a reason.
var claudemonBaseMarkers = []string{
	"CLAUDEMON_API_URL",
	"CLAUDEMON_HOOK_URL",
	"PORTS.claudemonApi",
	"PORTS.claudemonHook",
	"claudemonApi:",
	"claudemonHook:",
	"127.0.0.1:7891",
	"127.0.0.1:7890",
	"{hook_port}",
	"{api_base}",
	"{daemon_ws}",
	"newClaudemonClient",
	"WKS_CLAUDEMON_URL",
	"self.api_url",
}

// ---- the guards --------------------------------------------------------

// TestClaudemonRouteFixtureMatchesTheServedRegistry ties the fixture the Rust
// and Go sides both read to this package's own scan of the routers. Two
// derivations of one table: the Rust test extracts it from api.rs/hook.rs at
// compile time, and httproutes_test.go scans the same files here. If they ever
// disagree, one of them has stopped reading the router.
func TestClaudemonRouteFixtureMatchesTheServedRegistry(t *testing.T) {
	fixture := loadClaudemonRoutes(t)
	if len(fixture) < claudemonRouteFloor {
		t.Fatalf("contracts/claudemon-routes.json holds %d routes (floor %d) — it shrank, and every caller guard that reads it is now checking against a smaller table", len(fixture), claudemonRouteFloor)
	}

	inRegistry := map[string]bool{}
	for _, r := range HTTPRoutes() {
		if strings.HasPrefix(r.Server, "claudemon-") {
			inRegistry[r.Server+" "+r.Pattern] = true
		}
	}
	inFixture := map[string]bool{}
	for _, r := range fixture {
		inFixture[r.Server+" "+r.Pattern] = true
	}
	for k := range inFixture {
		if !inRegistry[k] {
			t.Errorf("contracts/claudemon-routes.json carries %s, which internal/capspec/httproutes.go does not classify. The fixture is regenerated from the routers (`make claudemon-routes`); a row here with no registry row means the route shipped unclassified.", k)
		}
	}
	for k := range inRegistry {
		if !inFixture[k] {
			t.Errorf("httproutes.go classifies %s and contracts/claudemon-routes.json does not serve it. Run `make claudemon-routes`: either the fixture is stale, or the route was deleted and its classification outlived it — which is exactly how the /git family stayed 'documented' after 37320188 removed it.", k)
		}
	}
}

// TestEveryClaudemonCallerPathIsServed is the sweep. It is the thing that would
// have gone red the day the /git routes were deleted.
func TestEveryClaudemonCallerPathIsServed(t *testing.T) {
	routes := loadClaudemonRoutes(t)
	total := 0
	for _, c := range claudemonCallers {
		paths := scanCallerPaths(t, c)
		if len(paths) < c.floor {
			t.Errorf("%s: the scan found %d claudemon paths (floor %d) — %s. A caller scan that stops matching reports a clean sweep over nothing, which is the failure this whole file exists to refuse.", c.file, len(paths), c.floor, c.what)
			continue
		}
		total += len(paths)
		for _, p := range paths {
			if routeServing(routes, c.server, p.path) == nil {
				t.Errorf("%s:%d builds %s %q and claudemon serves no such route.\n  caller: %s\n  This is the /git failure verbatim: the request 404s, the caller's error path is usually a swallowed `if let Ok(..)` / `.catch(() => {})`, and no test can see it because the caller's own mock answers any path. Fix the caller, or restore the route — do NOT widen this table.",
					c.file, p.line, c.server, p.path, c.what)
			}
		}
	}
	if total < 60 {
		t.Fatalf("the whole sweep matched only %d caller paths across %d files — the extraction has broken and this guard is comparing almost nothing", total, len(claudemonCallers))
	}
}

// TestEveryClaudemonCallerFileIsEnumerated holds the list above to the repo.
//
// This is the guard headless_completeness_test.go does not have: it enumerates
// three client files by name and nothing notices when a fourth appears (wks-tui
// is a bus client too, and is not on its list). An enumeration that can silently
// miss a client is the same blind spot as the seam it is guarding.
func TestEveryClaudemonCallerFileIsEnumerated(t *testing.T) {
	root := repoRootOrSkip(t)

	declared := map[string]bool{}
	for _, c := range claudemonCallers {
		declared[c.file] = true
	}

	found := 0
	err := filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if d.IsDir() {
			switch d.Name() {
			case "node_modules", "target", "dist", "release", ".git", "vendor", "out":
				return filepath.SkipDir
			}
			return nil
		}
		switch filepath.Ext(path) {
		case ".rs", ".ts", ".tsx", ".go":
		default:
			return nil
		}
		rel := filepath.ToSlash(strings.TrimPrefix(path, root+string(filepath.Separator)))
		// A test file's URL is asserted against its own mock, which is the
		// pinning-your-own-URL problem this work exists to remove — and the fix
		// for THAT is the mock answering 404 (apps/tui/src/claudemon.rs), not an
		// enumeration entry per test.
		if strings.HasSuffix(rel, "_test.go") || strings.Contains(rel, ".test.") || strings.Contains(rel, "/tests/") {
			return nil
		}
		body, readErr := os.ReadFile(path)
		if readErr != nil {
			return nil
		}
		src := string(body)
		for _, m := range claudemonBaseMarkers {
			if !strings.Contains(src, m) {
				continue
			}
			found++
			if declared[rel] {
				return nil
			}
			if _, ok := claudemonNonCallers[rel]; ok {
				return nil
			}
			t.Errorf("%s reaches for a claudemon listener (it contains %q) and is neither a declared caller in claudemonCallers nor a declared non-caller in claudemonNonCallers.\n  Add it to one of the two. If it builds request paths, the sweep must see them; if it does not, say in one sentence what it does with the base instead. A client nobody enumerated is exactly how six dead /git calls survived six weeks.", rel, m)
			return nil
		}
		return nil
	})
	if err != nil {
		t.Fatalf("walk: %v", err)
	}
	if found < len(claudemonCallers) {
		t.Fatalf("the repo-wide scan matched only %d files against %d markers, fewer than the %d callers already declared — the markers have stopped matching and this guard is enumerating nothing", found, len(claudemonBaseMarkers), len(claudemonCallers))
	}
	// And the list may not name a file that is gone: a caller row pointing at a
	// deleted path silently stops scanning, which reads as "clean".
	for _, c := range claudemonCallers {
		if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(c.file))); err != nil {
			t.Errorf("claudemonCallers names %s, which does not exist: %v. A row that cannot be read scans zero paths and passes.", c.file, err)
		}
	}
	for rel := range claudemonNonCallers {
		if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(rel))); err != nil {
			t.Errorf("claudemonNonCallers names %s, which does not exist: %v. Delete the row.", rel, err)
		}
	}
}

// TestEveryClaudemonRouteHasACallerOrADeclaredReason is the other direction: a
// route the daemon serves and nothing calls. It is not automatically a bug — the
// hook routes are called by Claude Code's own shellouts and the public REST page
// documents the API for third parties — but it is a decision, and an undeclared
// orphan is a route whose deletion nobody would notice either.
func TestEveryClaudemonRouteHasACallerOrADeclaredReason(t *testing.T) {
	routes := loadClaudemonRoutes(t)
	called := map[string]bool{}
	for _, c := range claudemonCallers {
		for _, p := range scanCallerPaths(t, c) {
			if r := routeServing(routes, c.server, p.path); r != nil {
				called[r.Server+" "+r.Pattern] = true
			}
		}
	}
	if len(called) < 20 {
		t.Fatalf("only %d routes were reached by the caller sweep — the extraction has broken, and every route would now look orphaned", len(called))
	}
	for _, r := range routes {
		key := r.Server + " " + r.Pattern
		if called[key] {
			if reason, ok := claudemonRouteCallers[key]; ok {
				t.Errorf("%s is declared caller-less (%q) and the sweep found a caller for it. Delete the row — a stale declaration hides the next real orphan.", key, reason)
			}
			continue
		}
		if reason, ok := claudemonRouteCallers[key]; ok {
			if len(strings.TrimSpace(reason)) < 60 {
				t.Errorf("%s is declared caller-less with no real reason (%q). Say who calls it from outside the repo, or say it is dead.", key, reason)
			}
			continue
		}
		t.Errorf("claudemon serves %s and nothing in this repo calls it.\n  That is either an external caller (Claude Code's hooks, a third party reading landing/build-client.html) or dead weight, and which one has to be written down: add a row to claudemonRouteCallers with the reason, or delete the route. A route with no caller and no reason is one nobody would miss — which is the same blindness as a caller with no route, pointing the other way.", key)
	}
}

// ---- scanning ----------------------------------------------------------

type callerPath struct {
	path string
	line int
}

func scanCallerPaths(t *testing.T, c callerScan) []callerPath {
	t.Helper()
	src := string(mustReadRepoFile(t, splitPath(c.file)...))
	if c.cutRustTests {
		if i := strings.Index(src, "#[cfg(test)]"); i > 0 {
			src = src[:i]
		}
	}
	if strings.HasSuffix(c.file, ".go") {
		src = goConcatChains(src)
	}
	var out []callerPath
	seen := map[string]bool{}
	for _, re := range c.res {
		for _, loc := range re.FindAllStringSubmatchIndex(src, -1) {
			raw := src[loc[2]:loc[3]]
			p := normalizeCallerPath(raw + c.suffix)
			if p == "" {
				continue
			}
			line := 1 + strings.Count(src[:loc[0]], "\n")
			key := fmt.Sprintf("%s@%d", p, line)
			if seen[key] {
				continue
			}
			seen[key] = true
			out = append(out, callerPath{path: p, line: line})
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].line < out[j].line })
	return out
}

var (
	// A Go `"…" + expr + "…"` pair. Applied to a fixed point so a three-literal
	// chain folds too.
	goPairRe = regexp.MustCompile(`"([^"\n]*)"\s*\+\s*[^"\n+,]+?\s*\+\s*"([^"\n]*)"`)
	// A Go literal ending in `/` with a value concatenated after it —
	// `getRaw(ctx, "/sessions/"+id)`.
	goTailRe = regexp.MustCompile(`"([^"\n]*/)"\s*\+\s*([A-Za-z_][\w.]*(?:\([^()]*\))?)`)
)

// goConcatChains folds Go string concatenation into single literals so one
// regex can read a path the source spells in five pieces. Line numbers survive
// because every replacement stays on its own line.
func goConcatChains(src string) string {
	for i := 0; i < 8; i++ {
		next := goPairRe.ReplaceAllString(src, `"$1:param$2"`)
		if next == src {
			break
		}
		src = next
	}
	return goTailRe.ReplaceAllString(src, `"$1:param"`)
}

// normalizeCallerPath turns a caller's spelling into a comparable path.
//
// The one rule with teeth is what to do with an interpolation hole — `{id}`,
// `${sessionId}`, `{}`, or the `:param` goConcatChains already folded. A hole
// that occupies a WHOLE segment is a path parameter and becomes `:param`. A hole
// that starts mid-segment is not: it is a query string or a conditional suffix
// the caller composes (`…/transcript${qs}`, `…/models${qs ? \`?${qs}\` : ”}`),
// and the honest answer is that the path ENDS there. Folding those into a
// segment invented `/sessions/:id/transcript:param`, a route nothing serves —
// a false positive, which on a guard like this is worse than a miss: it teaches
// the next person to widen the table.
func normalizeCallerPath(raw string) string {
	var b strings.Builder
	for i := 0; i < len(raw); {
		if raw[i] == '{' || (raw[i] == '$' && i+1 < len(raw) && raw[i+1] == '{') {
			open := i
			if raw[open] == '$' {
				open++
			}
			k := strings.IndexByte(raw[open:], '}')
			if k < 0 {
				break // unterminated: the rest is not a path
			}
			end := open + k + 1
			atStart := i == 0 || raw[i-1] == '/'
			atEnd := end == len(raw) || raw[end] == '/'
			if !atStart || !atEnd {
				break
			}
			b.WriteString(":param")
			i = end
			continue
		}
		if !isPathByte(raw[i]) {
			break
		}
		b.WriteByte(raw[i])
		i++
	}
	p := b.String()
	if i := strings.IndexAny(p, "?#"); i >= 0 {
		p = p[:i]
	}
	if !strings.HasPrefix(p, "/") {
		return ""
	}
	if len(p) > 1 {
		p = strings.TrimSuffix(p, "/")
	}
	return p
}

// isPathByte is the character set a URL path may use here. Anything else — a
// backtick, a paren, a space — ends the path, because it is the caller's source
// syntax leaking in rather than part of the request.
func isPathByte(c byte) bool {
	switch {
	case c >= 'a' && c <= 'z', c >= 'A' && c <= 'Z', c >= '0' && c <= '9':
		return true
	}
	return strings.IndexByte("/-_.~:?#=&%", c) >= 0
}

// routeServing resolves a caller path the way axum would: segment by segment,
// with a static segment beating a wildcard one.
func routeServing(routes []fixtureRoute, server, path string) *fixtureRoute {
	want := strings.Split(strings.TrimPrefix(path, "/"), "/")
	var wildcard *fixtureRoute
	for i := range routes {
		r := routes[i]
		if r.Server != server {
			continue
		}
		have := strings.Split(strings.TrimPrefix(r.Pattern, "/"), "/")
		if len(have) != len(want) {
			continue
		}
		exact := true
		ok := true
		for j := range have {
			switch {
			case strings.HasPrefix(have[j], ":"):
				exact = false
			case have[j] == want[j]:
			default:
				ok = false
			}
			if !ok {
				break
			}
		}
		if !ok {
			continue
		}
		if exact {
			return &routes[i]
		}
		if wildcard == nil {
			wildcard = &routes[i]
		}
	}
	return wildcard
}

func loadClaudemonRoutes(t *testing.T) []fixtureRoute {
	t.Helper()
	var doc struct {
		Routes []fixtureRoute `json:"routes"`
	}
	if err := json.Unmarshal(mustReadRepoFile(t, claudemonRoutesFixture...), &doc); err != nil {
		t.Fatalf("contracts/claudemon-routes.json does not parse: %v", err)
	}
	if len(doc.Routes) == 0 {
		t.Fatalf("contracts/claudemon-routes.json has no routes — every guard reading it would pass over an empty table")
	}
	return doc.Routes
}

func repoRootOrSkip(t *testing.T) string {
	t.Helper()
	root, err := sweepguard.Root()
	if err != nil {
		t.Skipf("not a monorepo checkout, so the repo-wide caller scan has nothing to walk: %v", err)
	}
	return root
}
