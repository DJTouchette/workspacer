package main

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

// THE HOOK-REGISTRATION STEP.
//
// `claudemon init` merges claudemon's hook + statusLine forwarders into
// ~/.claude/settings.json. The desktop runs it on every boot
// (apps/desktop/src/main/index.ts, runClaudemonInit) and `workspacer serve` did
// not — which is invisible on a developer machine, because the desktop already
// wrote those hooks into the same shared file. On a state directory where the
// desktop has never run (a container, a fresh volume, a CI box) nothing
// registers them, and then:
//
//   - a PTY session never receives a hook, so it stays SessionMode::Unknown
//     forever (services/claudemon/src/session/state.rs — Unknown is the default
//     and only hooks set Input/Responding); and
//   - a spawn's `first_message` is held until the `Input` transition
//     (services/claudemon/src/session/store.rs, queue_first_message +
//     schedule_pending_flush), so a DISPATCHED WORKER NEVER RECEIVES ITS PROMPT.
//     It sits at an empty composer looking alive.
//
// fleet.quiescence is NOT the casualty: `mode: "unknown"` is not a resting mode
// in internal/quiescence (stateBlocker returns KindSessionUnknown), so a
// hookless PTY session BLOCKS quiescence rather than reading as idle. That
// failure is in the safe direction — the machine stays awake — which is why
// this went unnoticed.

// fakeDaemon writes a shell script that appends its own argv to logPath. `init`
// is one-shot (it exits, like the real subcommand); anything else blocks, so the
// supervisor sees a live child rather than a crash loop.
func fakeDaemon(t *testing.T, dir, name, logPath string) string {
	t.Helper()
	p := filepath.Join(dir, name)
	script := fmt.Sprintf(`#!/bin/sh
printf '%%s %%s\n' "%s" "$*" >> %q
case "$1" in
  init) exit 0 ;;
esac
while : ; do sleep 1 ; done
`, name, logPath)
	if err := os.WriteFile(p, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return p
}

// freePorts returns n ports nothing is listening on right now.
func freePorts(t *testing.T, n int) []int {
	t.Helper()
	var out []int
	var keep []net.Listener
	for i := 0; i < n; i++ {
		l, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		keep = append(keep, l)
		out = append(out, l.Addr().(*net.TCPAddr).Port)
	}
	for _, l := range keep {
		l.Close()
	}
	return out
}

// TestServeRegistersClaudeCodeHooksBeforeStartingTheDaemons drives the real
// bootStack against stand-in binaries and asserts the hook registration ran, and
// ran FIRST. Ordering matters: init only rewrites a JSON file, but a session
// spawned in the window before it lands would come up hookless, so it belongs
// ahead of the daemons rather than beside them.
//
// The stack never becomes healthy (the fakes bind nothing) — that is fine and
// deliberate: a short context makes waitForHealth give up promptly, and the call
// log has already recorded what was launched.
func TestServeRegistersClaudeCodeHooksBeforeStartingTheDaemons(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("the stand-in daemons are /bin/sh scripts")
	}
	dir := t.TempDir()
	logPath := filepath.Join(dir, "calls.log")
	ports := freePorts(t, 3)

	opts := serveOptions{
		Host:         "127.0.0.1",
		HubPort:      ports[0],
		APIPort:      ports[1],
		HookPort:     ports[2],
		Token:        "tok",
		ClaudemonBin: fakeDaemon(t, dir, "claudemon", logPath),
		HubBin:       fakeDaemon(t, dir, "hub", logPath),
	}

	ctx, cancel := context.WithTimeout(context.Background(), 1500*time.Millisecond)
	defer cancel()
	if stk, err := bootStack(ctx, opts, os.Stderr); err == nil {
		stk.shutdown(os.Stderr)
	}
	// Give the one-shot init's append a moment to land even if bootStack bailed
	// on the health wait first.
	deadline := time.Now().Add(2 * time.Second)
	var calls []string
	for {
		b, _ := os.ReadFile(logPath)
		calls = nil
		for _, line := range strings.Split(strings.TrimSpace(string(b)), "\n") {
			if strings.TrimSpace(line) != "" {
				calls = append(calls, strings.TrimSpace(line))
			}
		}
		if len(calls) > 0 || time.Now().After(deadline) {
			break
		}
		time.Sleep(50 * time.Millisecond)
	}

	var initLine string
	initIndex := -1
	for i, c := range calls {
		if strings.HasPrefix(c, "claudemon init") {
			initLine, initIndex = c, i
			break
		}
	}
	if initIndex < 0 {
		t.Fatalf("`workspacer serve` never ran `claudemon init` — on a fresh state dir the "+
			"Claude Code hooks go unregistered, so a PTY session stays mode=unknown and a "+
			"dispatched worker's first message is never delivered.\ncalls:\n  %s",
			strings.Join(calls, "\n  "))
	}
	if initIndex != 0 {
		t.Errorf("hook registration ran at position %d, want first (a session spawned before it "+
			"lands comes up hookless)\ncalls:\n  %s", initIndex, strings.Join(calls, "\n  "))
	}
	if want := fmt.Sprintf("--hook-port %d", opts.HookPort); !strings.Contains(initLine, want) {
		t.Errorf("init argv = %q, want it to carry %q — the hooks it writes are a curl at that "+
			"port, so a wrong one registers a forwarder that posts into nothing", initLine, want)
	}
}

// The escape hatch: an operator who manages ~/.claude/settings.json themselves
// (or ships it read-only in an image) must be able to turn the step off.
func TestNoClaudemonInitSkipsTheStep(t *testing.T) {
	opts := serveOptions{
		Host: "127.0.0.1", HubPort: 7895, APIPort: 7891, HookPort: 7890,
		Token: "tok", ClaudemonBin: "/bin/claudemon", HubBin: "/bin/hub",
		SkipClaudemonInit: true,
	}
	if got := buildServePlan(opts).Init.Bin; got != "" {
		t.Errorf("plan.Init.Bin = %q with --no-claudemon-init, want empty (no step)", got)
	}
}

// The plan-level companion: the step is part of the launch plan, so it is
// table-visible next to the two daemons rather than buried in bootStack.
func TestBuildServePlanCarriesTheInitStep(t *testing.T) {
	opts := serveOptions{
		Host: "127.0.0.1", HubPort: 7895, APIPort: 7891, HookPort: 7890,
		Token: "tok", ClaudemonBin: "/bin/claudemon", HubBin: "/bin/hub",
	}
	p := buildServePlan(opts)
	if p.Init.Bin != "/bin/claudemon" {
		t.Errorf("plan.Init.Bin = %q, want the claudemon binary", p.Init.Bin)
	}
	if len(p.Init.Args) == 0 || p.Init.Args[0] != "init" {
		t.Errorf("plan.Init.Args = %v, want it to start with `init`", p.Init.Args)
	}
	if got := argsAfter(p.Init.Args, "--hook-port"); got != "7890" {
		t.Errorf("init --hook-port = %q, want the same port claudemon serves hooks on", got)
	}
}
