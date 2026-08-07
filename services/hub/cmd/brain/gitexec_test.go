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
	write := func(rel, contents string) {
		t.Helper()
		params := map[string]string{"path": filepath.Join(lib, rel), "contents": contents}
		raw, _ := json.Marshal(params)
		if _, err := reg.handle(context.Background(), "fs.write", raw); err != nil {
			t.Fatalf("fs.write %s: %v (the attack needs these writes to be ALLOWED)", rel, err)
		}
	}
	// A minimal repository skeleton, all of it through fs.write.
	write(".git/HEAD", "ref: refs/heads/main\n")
	write(".git/refs/heads/.keep", "")
	write(".git/objects/info/packs", "")
	write("a.txt", "hello")
	write(".git/config", "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = \"sh -c 'touch "+marker+"; echo'\"\n")

	raw, _ := json.Marshal(map[string]string{"path": lib})
	if _, err := reg.handle(context.Background(), "fs.listEntries", raw); err != nil {
		t.Fatalf("fs.listEntries on a config store must be allowed: %v", err)
	}
	if _, err := os.Stat(marker); err == nil {
		t.Fatalf("ARBITRARY COMMAND EXECUTED: %s exists after fs.write + fs.listEntries", marker)
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
