package main

// COMPLETENESS OF THE HEADLESS CONFIGURATION.
//
// `workspacer serve` runs the hub with --brain-scope full (cmd/workspacer/
// plan.go) and serves the built web renderer at /app — advertised as full
// remote parity — plus the /m mobile PWA. In that configuration the providers
// are exactly: this brain's full scope, plus the handful of capabilities the hub
// answers in-process. There is no desktop.
//
// TestEveryDelegatedCapabilityHasABrainProvider does this for the DELEGATED
// configuration. Nothing did it for the headless one, so a client could call a
// method that has no provider anywhere and the only symptom was a per-call error
// string — which several call sites swallow into an empty list, so the phone's
// resume-a-recent-agent list renders empty as if there were no history.
//
// This does not demand that every gap be closed. It demands that every gap be
// KNOWN: an entry in headlessGaps, with what the user actually sees.

import (
	"regexp"
	"sort"
	"strings"
	"testing"
)

var (
	// `client.call<T>('x', …)` in the web backend (the type argument may span a
	// line and may itself contain parentheses, e.g. `import('…').GitStatus`),
	// and `call('x', …)` / `busCall('x', …)` in the hand-written mobile and
	// remote clients.
	clientCallRe = regexp.MustCompile(`(?s)(?:\bclient\s*\.\s*call|\bbusCall|\bcall)(?:<.*?>)?\(\s*'([a-z][\w.]*\.[\w.]+)'`)
)

// headlessGaps are the methods a shipped client can call that NO provider
// answers under `workspacer serve`, each with the degradation the user gets.
// Adding one is a decision; the empty value is what this guard refuses.
var headlessGaps = map[string]string{
	// claude.setModel / setPermissionMode / setEffort / handoffBrief were here
	// and are now PROVIDED (livecontrol.go): the mode pill, the model switcher,
	// the effort control and cross-provider handoff work on a headless node.
	// setPermissionMode carries the spawn path's escalation clamp with it — a
	// bus caller can TIGHTEN a running agent's mode and cannot loosen it.
	//
	// The entries these replace were rewritten a moment earlier to describe the
	// loud-failure work on the two clients (both now surface the rejection
	// rather than swallowing it). That work is not wasted by this: it is what a
	// user sees when the DAEMON refuses a live switch, which is still a real
	// answer — ok:false with a reason, and the restart path behind it.
	//
	// claude.handoffAgentBrief stays a gap ON PURPOSE. It is not a relay like
	// its sibling: main/services/agentHandoff.ts injects a write-this-brief
	// instruction into the live agent, waits for the file to appear, and falls
	// back to the mechanical brief on timeout. That orchestration has no
	// claudemon endpoint behind it, so porting it means porting the whole
	// service, not registering a method.
	"claude.handoffAgentBrief": "the agent-AUTHORED handoff brief is unavailable; the deterministic one (claude.handoffBrief) works, so a handoff still succeeds with a mechanically composed brief",
	// fs.readImage was here and is now PROVIDED (readimage.go), so chat
	// thumbnails render on a headless node — with the twin's own inline-bytes
	// fallback rather than a decoded thumbnail, since there is no image decoder
	// in this daemon.
	//
	// fs.watch / fs.unwatch stay gaps. They are not a relay or a read: the
	// desktop's pair drives a live host watcher whose emit sink mirrors every
	// change onto the bus as `fs.changed`, so a headless counterpart is a
	// filesystem-watcher subsystem (a new dependency, a new published topic,
	// per-path lifecycle and teardown) rather than a handler. The degradation is
	// narrow and additive — the editor pane shows what it read, it just does not
	// notice an outside edit — which is why it did not outrank the agent-facing
	// set in this pass.
	"fs.watch":   "the editor pane does not live-reload; an outside edit is noticed on the next open, not live",
	"fs.unwatch": "no-op counterpart of fs.watch",
	// git.status / git.log / git.diff / git.numstat were here and are now
	// PROVIDED (git.go): the READ-ONLY half of the git surface was ported into
	// the brain so a remote node's branch chip, Review pane, rail widget,
	// per-turn line counts and project_status work headless.
	//
	// The four below stay gaps ON PURPOSE, not for want of effort. This brain is
	// the provider that runs on an internet-facing node; a read-only surface
	// cannot mutate or publish a repository, so a bus token cannot commit or
	// push from a machine the user is not sitting at. Agents on the node still
	// commit through their own Bash tool, which is how work actually lands
	// there — the UI buttons are convenience, not capability.
	"git.commitDiff":    "no diff for a past commit",
	"git.commitNumstat": "no stats for a past commit",
	"git.stage":         "staging is unavailable; deliberately not ported — see above",
	"git.unstage":       "unstaging is unavailable; deliberately not ported — see above",
	"git.commit":        "committing is unavailable; deliberately not ported — see above",
	"git.push":          "pushing is unavailable; deliberately not ported — see above",
	// sessions.recent was here and is now PROVIDED (recent.go): the Sessions
	// pane and the phone's resume list are answered from claudemon's own
	// resumable-row list, so an empty list finally means "no sessions" rather
	// than "no provider". (replay.* is desktop-IPC only — no shipped headless
	// client calls it, so it is not a gap here.)
	//
	// sessions.recent is PROVIDED too (recent.go). The entry it replaces
	// recorded that /app had just stopped swallowing the failure — the Sessions
	// pane now says it could not be READ rather than that there is none — and
	// that distinction still matters: with a provider in place, an empty list
	// finally means "no sessions" and an error means the daemon is unreachable.
	//
	// brief.archive and brief.check were never listed here, for the same reason
	// replay.* is not: no shipped client calls them (they are agent-facing MCP
	// tools), so TestHeadlessGapsAreReachableFromAShippedClient would refuse the
	// entries. Their absence was recorded in brief.go's header instead — and
	// that is exactly why the gap survived: this file's guards see what a
	// BROWSER calls, and the caller that lost those two verbs was an AGENT on
	// the node. Both are now PROVIDED (briefboard.go, briefcheck.go), and the
	// class of gap they represent has a guard of its own:
	// headlessport_test.go's TestEveryBriefVerbTheFacadeExposesHasABrainHandler
	// enumerates from the MCP facade and capspec rather than from a client.
}

func headlessProviders(t *testing.T) map[string]bool {
	t.Helper()
	set := map[string]bool{}
	r := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	for _, m := range r.methodsForScope("full") {
		set[m] = true
	}
	hubMain := string(mustReadRepoFile(t, "services", "hub", "cmd", "hub", "main.go"))
	locals := hubLocalRe.FindAllStringSubmatch(hubMain, -1)
	if len(locals) == 0 {
		t.Fatal("parsed no RegisterLocal names from cmd/hub/main.go — the registration syntax changed; update hubLocalRe")
	}
	for _, m := range locals {
		set[m[1]] = true
	}
	return set
}

func clientCalls(t *testing.T, parts ...string) []string {
	t.Helper()
	body := string(mustReadRepoFile(t, parts...))
	seen := map[string]bool{}
	var out []string
	for _, m := range clientCallRe.FindAllStringSubmatch(body, -1) {
		if !seen[m[1]] {
			seen[m[1]] = true
			out = append(out, m[1])
		}
	}
	sort.Strings(out)
	return out
}

// Every method a shipped headless client can invoke either has a provider under
// `workspacer serve`, or is a KNOWN gap with the user-visible consequence
// written down.
func TestEveryHeadlessClientCallHasAProviderOrADeclaredGap(t *testing.T) {
	provided := headlessProviders(t)
	clients := map[string][]string{
		"/app (webBackend.ts)":  clientCalls(t, "apps", "desktop", "src", "renderer", "src", "backend", "webBackend.ts"),
		"/m (mobile.html)":      clientCalls(t, "services", "hub", "cmd", "hub", "mobile.html"),
		"/remote (remote.html)": clientCalls(t, "services", "hub", "cmd", "hub", "remote.html"),
	}
	for name, calls := range clients {
		if len(calls) == 0 {
			t.Fatalf("%s: parsed no capability calls — the call syntax changed; update clientCallRe", name)
		}
		for _, m := range calls {
			if provided[m] {
				continue
			}
			if why, known := headlessGaps[m]; known {
				if strings.TrimSpace(why) == "" {
					t.Errorf("headlessGaps[%q] has no reason — say what the user sees", m)
				}
				continue
			}
			t.Errorf("%s calls %q and NOTHING provides it under `workspacer serve` (brain full scope + the hub's own handlers). "+
				"Either implement it in the brain, or add it to headlessGaps with the degradation the user gets.", name, m)
		}
	}
}

// A declared gap that is actually provided now is a stale silencer: it would
// keep a real future gap quiet.
func TestHeadlessGapsAreStillGaps(t *testing.T) {
	provided := headlessProviders(t)
	for m := range headlessGaps {
		if provided[m] {
			t.Errorf("headlessGaps names %q, which the headless configuration now PROVIDES — drop the entry so it cannot hide a real gap later", m)
		}
	}
}

// And the gaps must be reachable from a client at all: an entry for a method
// nobody calls is noise that outlives its reason.
func TestHeadlessGapsAreReachableFromAShippedClient(t *testing.T) {
	called := map[string]bool{}
	for _, parts := range [][]string{
		{"apps", "desktop", "src", "renderer", "src", "backend", "webBackend.ts"},
		{"services", "hub", "cmd", "hub", "mobile.html"},
		{"services", "hub", "cmd", "hub", "remote.html"},
	} {
		for _, m := range clientCalls(t, parts...) {
			called[m] = true
		}
	}
	for m := range headlessGaps {
		if !called[m] {
			t.Errorf("headlessGaps names %q, which no shipped client calls — drop the entry", m)
		}
	}
}
