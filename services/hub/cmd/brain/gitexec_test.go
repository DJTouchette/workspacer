package main

// fs.listEntries must not be a command-execution primitive.
//
// The gitignore filter shells out to `git check-ignore` with cmd.Dir set to the
// directory being listed. git discovers a repository AT that directory and reads
// its `.git/config` — and `core.fsmonitor` in a git config is a command git
// RUNS. Every directory on this surface is writable by the same bus client that
// asks for the listing: `<configDir>/library` is a configStoreRoot and
// writeHostFile creates missing parents, so `fs.write` alone can mint a whole
// `.git` skeleton with zero live agents and no spawn capability, and then one
// `fs.listEntries` executes arbitrary shell as the daemon user.
//
// The probe below drives the SHIPPING handlers end to end and asserts the marker
// file does not appear.

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// gitAvailable reports whether the `git` binary exists; without it the probe is
// vacuous (gitIgnored short-circuits on exit 128) and must skip, not pass.
func gitAvailable(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not on PATH")
	}
}

func TestListEntriesDoesNotExecuteGitConfigCommands(t *testing.T) {
	gitAvailable(t)
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", filepath.Join(sandbox, "home"))
	t.Setenv("USERPROFILE", filepath.Join(sandbox, "home"))
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	resetCwdCacheForTest()

	// The listed directory is <configDir>/library — a configStoreRoot, so every
	// write below is a call the guard ALLOWS, with zero live agents.
	lib := filepath.Join(configDir(), "library")
	marker := filepath.Join(sandbox, "PWNED")

	reg := newRegistry(nil)
	// The skeleton is planted with direct writes, NOT through fs.write: the
	// guard now refuses every caller-supplied path that traverses a `.git`
	// component (TestWritingIntoAGitDirectoryIsRefused below is that half). This
	// probe is the residual case — a repository that exists for its own reasons,
	// with a config this process did not write — and the `-c` prefix is what has
	// to hold there.
	plant := func(rel, contents string) {
		t.Helper()
		full := filepath.Join(lib, rel)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(full, []byte(contents), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	plant(".git/HEAD", "ref: refs/heads/main\n")
	plant(".git/refs/heads/.keep", "")
	plant(".git/objects/info/packs", "")
	plant("a.txt", "hello")
	plant(".git/config", "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = \"sh -c 'touch "+marker+"; echo'\"\n")

	raw, _ := json.Marshal(map[string]string{"path": lib})
	if _, err := reg.handle(context.Background(), "fs.listEntries", raw); err != nil {
		t.Fatalf("fs.listEntries on a config store must be allowed: %v", err)
	}
	if _, err := os.Stat(marker); err == nil {
		t.Fatalf("ARBITRARY COMMAND EXECUTED: %s exists after fs.listEntries", marker)
	}
}

// The other half, and the one that closes the keys a `-c` prefix cannot reach.
//
// `filter.<drv>.clean` — which `git add` runs, i.e. git.stage — and
// `diff.<drv>.command` / `merge.<drv>.driver` / `trailer.<t>.command` are all
// NAMESPACED by a driver name the attacker chooses, so no fixed list of `-c`
// overrides can neutralize them. What they have in common is that the driver has
// to be DEFINED in a config file inside the repository's `.git` directory. So
// the guard refuses caller-supplied paths that traverse `.git` at all, and the
// definition can never be written.
//
// <configDir>/library is a configStoreRoot with zero live agents, and
// writeHostFile MkdirAll's the parents — the cheapest possible reach.
func TestWritingIntoAGitDirectoryIsRefused(t *testing.T) {
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", filepath.Join(sandbox, "home"))
	t.Setenv("USERPROFILE", filepath.Join(sandbox, "home"))
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	resetCwdCacheForTest()

	lib := filepath.Join(configDir(), "library")
	reg := newRegistry(nil)

	// Sanity: an ordinary file in the same directory IS writable, or the case
	// below proves nothing about `.git` in particular.
	raw, _ := json.Marshal(map[string]string{"path": filepath.Join(lib, "ok.txt"), "contents": "hi"})
	if _, err := reg.handle(context.Background(), "fs.write", raw); err != nil {
		t.Fatalf("control: an ordinary write inside a config store must be allowed: %v", err)
	}

	for _, rel := range []string{
		".git/config",           // filter.<drv>.clean / diff.<drv>.command live here
		".git/config.worktree",  // same file, per-worktree spelling
		".git/info/attributes",  // the attribute half of the filter chain
		".git/hooks/pre-commit", // an executable git runs on the next commit
		".GIT/config",           // APFS/NTFS open .git when handed .GIT
		"proj/.git/config",      // an interior component, not just the first
		".git",                  // the gitfile pointer form: `gitdir: /elsewhere`
	} {
		t.Run(rel, func(t *testing.T) {
			raw, _ := json.Marshal(map[string]string{
				"path":     filepath.Join(lib, rel),
				"contents": "[filter \"evil\"]\n\tclean = \"sh -c 'id > /tmp/PWNED'; cat\"\n",
			})
			if _, err := reg.handle(context.Background(), "fs.write", raw); err == nil {
				t.Fatalf("fs.write %s was ALLOWED — a caller-written git config is command execution", rel)
			}
			// And the read direction, because a .git/config carries remote URLs
			// with embedded tokens and the name of a credential store.
			raw, _ = json.Marshal(map[string]string{"path": filepath.Join(lib, rel)})
			if _, err := reg.handle(context.Background(), "fs.read", raw); err == nil {
				t.Fatalf("fs.read %s was ALLOWED", rel)
			}
		})
	}
}

// The exec-key list is a TWIN of GIT_NO_EXEC_CONFIG in
// apps/desktop/src/main/lib/gitExec.ts. The two providers answer the same bus
// methods, so a key neutralized on one side and not the other is a capability
// that executes commands depending on who happened to answer.
func TestGitNoExecKeysMatchTheDesktopTwin(t *testing.T) {
	src, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "apps", "desktop", "src", "main", "lib", "gitExec.ts"))
	if err != nil {
		t.Skipf("desktop twin not present in this checkout: %v", err)
	}
	ts := map[string]bool{}
	for _, line := range strings.Split(string(src), "\n") {
		line = strings.TrimSpace(strings.Trim(strings.TrimSpace(line), ","))
		if !strings.HasPrefix(line, "'") || !strings.HasSuffix(line, "'") {
			continue
		}
		v := strings.Trim(line, "'")
		if v == "-c" || !strings.Contains(v, "=") {
			continue
		}
		ts[v] = true
	}
	if len(ts) == 0 {
		t.Fatal("parsed zero keys out of gitExec.ts — this parity test has stopped comparing anything")
	}
	for _, kv := range gitNoExecKeys {
		if !ts[kv] {
			t.Errorf("gitNoExecConfig() neutralizes %q and the desktop twin does not", kv)
		}
		delete(ts, kv)
	}
	for kv := range ts {
		t.Errorf("gitExec.ts neutralizes %q and gitNoExecConfig() does not", kv)
	}
}

// The static half: a `git` invocation that forgets the prefix is the same bug
// again in a different subcommand, and a behavioural probe per call site does
// not scale. Every exec.Command("git", ...) in the brain must start from
// gitNoExecConfig().
func TestEveryGitInvocationCarriesTheNoExecPrefix(t *testing.T) {
	if got := gitNoExecConfig(); len(got) < 2 || got[0] != "-c" || !strings.HasPrefix(got[1], "core.fsmonitor=") {
		t.Fatalf("gitNoExecConfig() = %v, want it to start with -c core.fsmonitor=", got)
	}
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatal(err)
	}
	checked := 0
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".go") || strings.HasSuffix(e.Name(), "_test.go") {
			continue
		}
		src, err := os.ReadFile(e.Name())
		if err != nil {
			t.Fatal(err)
		}
		for _, line := range strings.Split(string(src), "\n") {
			if !strings.Contains(line, `exec.Command("git"`) && !strings.Contains(line, `exec.CommandContext(ctx, "git"`) {
				continue
			}
			checked++
			if !strings.Contains(line, "gitNoExecConfig") && !strings.Contains(line, "args...") {
				t.Errorf("%s: git invoked without gitNoExecConfig(): %s", e.Name(), strings.TrimSpace(line))
			}
		}
	}
	if checked == 0 {
		t.Fatal("swept zero git invocations — this guard has stopped guarding anything")
	}
}
