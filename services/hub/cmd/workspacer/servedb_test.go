package main

import (
	"context"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

// THE SESSION DATABASE IS NOT PART OF THE PLAN — AND IT MUST BE.
//
// claudemon's argv was `serve --host --hook-port --api-port`. Every port it
// binds was pinned; the one piece of PERSISTENT state it opens was not. With no
// --db-path, claudemon resolves it itself (store/mod.rs default_db_path):
// $XDG_DATA_HOME/claudemon/state.db, else ~/.claudemon/state.db, else the
// RELATIVE .claudemon/state.db under whatever the process CWD happens to be.
//
// Two consequences, both silent:
//
//   - A second stack started on alternate ports — the only way to run one beside
//     a live one, since bootStack refuses busy ports — opens the SAME
//     ~/.claudemon/state.db. The two share the `sessions` and `events` tables
//     outright: the newcomer's boot hydration (daemon/mod.rs
//     load_recent_sessions → store.hydrate) adopts the live stack's sessions as
//     resumable rows of its own, its clients can resume and act on them, and its
//     fleet.quiescence sampler counts them.
//   - With no resolvable home the database lands at a relative path under the
//     process CWD, which on a container is the rootfs that is rebuilt on every
//     start.
//
// Neither reports anything. So `serve` pins the path explicitly, exactly like
// the ports, and refuses the one combination that means "a second stack sharing
// one database".

// resolveDBPath is the whole decision, so it is tested as one — no ports bound,
// nothing spawned. (Binding claudemon's real default ports in a test would
// collide with whatever is genuinely running on the machine.)
func TestResolveDBPath(t *testing.T) {
	scratch := t.TempDir()

	t.Run("an explicit path is taken verbatim", func(t *testing.T) {
		want := filepath.Join(scratch, "mine", "state.db")
		got, err := resolveDBPath(serveOptions{
			DBPath: want, APIPort: 9991, HookPort: 9990,
		})
		if err != nil {
			t.Fatalf("an explicitly pinned database must be honoured on any ports: %v", err)
		}
		if got != want {
			t.Errorf("= %q, want %q", got, want)
		}
	})

	t.Run("default ports resolve the shared default", func(t *testing.T) {
		data := t.TempDir()
		t.Setenv("XDG_DATA_HOME", data)
		got, err := resolveDBPath(serveOptions{
			APIPort: defaultClaudemonAPIPort, HookPort: defaultClaudemonHookPort,
		})
		if err != nil {
			t.Fatal(err)
		}
		// The desktop app and `serve` deliberately share one session store —
		// they adopt each other rather than coexisting — so the default must
		// stay exactly where claudemon would have put it.
		if want := filepath.Join(data, "claudemon", "state.db"); got != want {
			t.Errorf("= %q, want %q — pinning must not RELOCATE an existing install's database", got, want)
		}
	})

	t.Run("alternate claudemon ports with no pinned database are refused", func(t *testing.T) {
		t.Setenv("XDG_DATA_HOME", t.TempDir())
		_, err := resolveDBPath(serveOptions{
			APIPort: 9991, HookPort: defaultClaudemonHookPort,
		})
		if err == nil {
			t.Fatal("allowed a second claudemon to open the shared default session database — " +
				"the two stacks share every session and event row, and neither says so")
		}
		for _, want := range []string{"--claudemon-db-path", "state.db"} {
			if !strings.Contains(err.Error(), want) {
				t.Errorf("error %q does not mention %q", err, want)
			}
		}
	})

	t.Run("an alternate hook port alone is enough to refuse", func(t *testing.T) {
		t.Setenv("XDG_DATA_HOME", t.TempDir())
		if _, err := resolveDBPath(serveOptions{
			APIPort: defaultClaudemonAPIPort, HookPort: 9990,
		}); err == nil {
			t.Error("only the API port was checked; either alternate port means a second daemon")
		}
	})

	t.Run("a relative derived path is refused, not silently accepted", func(t *testing.T) {
		// claudemon's third fallback is the RELATIVE ".claudemon/state.db",
		// which puts the session store under whatever the CWD happens to be —
		// on a container, the rootfs that is rebuilt on every start. Pinning
		// must not launder that into a real-looking absolute path either.
		t.Setenv("XDG_DATA_HOME", "relative/dir")
		if got, err := resolveDBPath(serveOptions{
			APIPort: defaultClaudemonAPIPort, HookPort: defaultClaudemonHookPort,
		}); err == nil {
			t.Errorf("accepted the relative path %q", got)
		} else if !strings.Contains(err.Error(), "XDG_DATA_HOME") {
			t.Errorf("error %q does not name the variable at fault", err)
		}
	})

	t.Run("no home and no XDG is an error, not a relative guess", func(t *testing.T) {
		if runtime.GOOS == "windows" {
			t.Skip("HOME is not how Windows resolves this")
		}
		t.Setenv("XDG_DATA_HOME", "")
		os.Unsetenv("XDG_DATA_HOME")
		t.Setenv("HOME", "")
		os.Unsetenv("HOME")
		// user.Current() may still answer from passwd, exactly as claudemon's
		// `directories` crate does — so accept either a real absolute path or a
		// named error, never a relative one.
		got, err := resolveDBPath(serveOptions{
			APIPort: defaultClaudemonAPIPort, HookPort: defaultClaudemonHookPort,
		})
		if err == nil && !filepath.IsAbs(got) {
			t.Errorf("= %q, want an absolute path or a clear error", got)
		}
		if err != nil && !strings.Contains(err.Error(), "--claudemon-db-path") {
			t.Errorf("error %q does not say how to fix it", err)
		}
	})
}

// The pinned path has to actually reach the daemon.
func TestBuildServePlanPinsTheDatabasePath(t *testing.T) {
	p := buildServePlan(serveOptions{
		Host: "127.0.0.1", HubPort: 7895, APIPort: 7891, HookPort: 7890,
		Token: "tok", ClaudemonBin: "/bin/claudemon", HubBin: "/bin/hub",
		DBPath: "/data/state/claudemon/state.db",
	})
	if got := argsAfter(p.Claudemon.Args, "--db-path"); got != "/data/state/claudemon/state.db" {
		t.Errorf("claudemon --db-path = %q — an unpinned database is resolved by the daemon "+
			"itself, so a second stack silently opens the live one", got)
	}
}

// End to end through the real bootStack: the resolved path lands in the argv of
// the process that actually opens the file. Alternate ports throughout (nothing
// on this machine's real ports is touched), with the database pinned so the
// guard is satisfied.
func TestServePassesTheResolvedDatabasePathToClaudemon(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("the stand-in daemons are /bin/sh scripts")
	}
	dir := t.TempDir()
	logPath := filepath.Join(dir, "calls.log")
	db := filepath.Join(dir, "scratch", "state.db")
	ports := freePorts(t, 3)

	opts := serveOptions{
		Host:         "127.0.0.1",
		HubPort:      ports[0],
		APIPort:      ports[1],
		HookPort:     ports[2],
		Token:        "tok",
		DBPath:       db,
		ClaudemonBin: fakeDaemon(t, dir, "claudemon", logPath),
		HubBin:       fakeDaemon(t, dir, "hub", logPath),
	}
	ctx, cancel := context.WithTimeout(context.Background(), 1500*time.Millisecond)
	defer cancel()
	if stk, err := bootStack(ctx, opts, os.Stderr); err == nil {
		stk.shutdown(os.Stderr)
	}

	line := waitForCall(t, logPath, "claudemon serve")
	if !strings.Contains(line, "--db-path "+db) {
		t.Errorf("claudemon argv = %q, want --db-path %s", line, db)
	}
}

// waitForCall polls the stand-in daemons' call log for a line with the prefix.
func waitForCall(t *testing.T, logPath, prefix string) string {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for {
		b, _ := os.ReadFile(logPath)
		for _, line := range strings.Split(string(b), "\n") {
			if strings.HasPrefix(strings.TrimSpace(line), prefix) {
				return strings.TrimSpace(line)
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("no %q line in the call log:\n%s", prefix, b)
			return ""
		}
		time.Sleep(50 * time.Millisecond)
	}
}
