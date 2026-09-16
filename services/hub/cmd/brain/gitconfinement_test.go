package main

// THE CONFINEMENT ON THE READ-ONLY git.* CAPABILITIES, driven through the real
// dispatch.
//
// The port is not the risky part; the guard is. `git.diff {untracked: true}`
// runs `git diff --no-index -- /dev/null <path>`, where git reads `path` as a
// plain FILESYSTEM operand — so it renders ANY readable file as an all-added
// diff: gitignored, untracked, and tracked-but-unmodified files alike, none of
// which a path-less `git.diff` returns. The desktop shipped that hole once
// (hubCapabilities.ts's git.diff comment records it: an agent cwd of
// <repo>/frontend read <repo>/backend/.env, and a $HOME that happened to be a
// dotfiles repo read ~/.ssh/id_rsa) and closed it by holding that one leg to
// the ordinary workspace roots on top of the work-tree root.
//
// The second trap is that the WORK-TREE ROOT is derived: it comes out of
// `rev-parse --show-toplevel` AFTER the cwd guard, and nothing checks it against
// the allow-list. Anchoring a pathspec on the caller's cwd instead would check a
// different file than git opens whenever the agent cwd is a subdirectory — the
// ordinary monorepo case — and trusting the root turns "a pathspec inside the
// confined repo" into "anything inside a repository that merely CONTAINS an
// allowed directory".
//
// So every test below attempts the thing a guard exists to prevent and asserts
// the refusal, with a control case in each file proving the guard does not
// simply deny everything (a handler that refuses unconditionally passes every
// deny case ever written).

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// secretMarker is the byte string that must never come back through a git
// capability. Every deny assertion greps the whole response for it, because
// "the call errored" and "the call did not leak" are two different claims and
// only the second one is the point.
const secretMarker = "AWS_SECRET_ACCESS_KEY=never-leak-me"

// gitFixture is one sandbox: a repository whose only allowed root is a
// SUBDIRECTORY of it, which is the shape both holes need.
type gitFixture struct {
	sandbox  string // the whole tree; nothing here is an allowed root by itself
	repo     string // the git work-tree root — DERIVED, never allow-listed
	agentCwd string // <repo>/frontend — the one live agent cwd
	outside  string // <sandbox>/outside — a plain directory, no repo, no root
}

// newGitFixture builds the tree and points HOME/config at throwaway directories
// so neither the runner's real home nor its real config dir can admit anything.
func newGitFixture(t *testing.T) gitFixture {
	t.Helper()
	gateGit(t)
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	setHome(t, filepath.Join(root, "home"))
	setConfigHome(t, filepath.Join(root, "config"))
	if err := os.MkdirAll(filepath.Join(root, "home"), 0o755); err != nil {
		t.Fatal(err)
	}
	resetCwdCacheForTest()
	t.Cleanup(resetCwdCacheForTest)

	fx := gitFixture{
		sandbox:  root,
		repo:     filepath.Join(root, "repo"),
		agentCwd: filepath.Join(root, "repo", "frontend"),
		outside:  filepath.Join(root, "outside"),
	}
	for _, d := range []string{fx.agentCwd, filepath.Join(fx.repo, "backend"), fx.outside} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	write := func(p, contents string) {
		t.Helper()
		if err := os.WriteFile(p, []byte(contents), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write(filepath.Join(fx.agentCwd, "tracked.ts"), "export const a = 1;\n")
	write(filepath.Join(fx.repo, "backend", "tracked.go"), "package backend\n")
	// The two files the guard exists to keep out of a diff. Both are readable by
	// this process and both are refused to a bus caller by fs.read.
	write(filepath.Join(fx.repo, "backend", ".env"), secretMarker+"\n")
	write(filepath.Join(fx.outside, "secret.txt"), secretMarker+"\n")

	gitInit(t, fx.repo)
	// An untracked file INSIDE the allowed root: the control case, and the
	// legitimate reason `untracked` exists at all.
	write(filepath.Join(fx.agentCwd, "untracked.txt"), "brand new line\n")
	return fx
}

// gitInit makes `dir` a repository with one commit. Deliberately not routed
// through runGit: a test that builds its fixture with the code under test can
// only ever agree with it.
func gitInit(t *testing.T, dir string) {
	t.Helper()
	run := func(args ...string) {
		t.Helper()
		cmd := exec.Command("git", args...)
		cmd.Dir = dir
		// GIT_CONFIG_NOSYSTEM keeps /etc/gitconfig out; HOME is already the
		// sandbox's, so the user's own ~/.gitconfig is out too.
		cmd.Env = append(os.Environ(), "GIT_CONFIG_NOSYSTEM=1")
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	run("init", "-q")
	run("config", "user.email", "test@example.invalid")
	run("config", "user.name", "Test")
	run("config", "commit.gpgsign", "false")
	run("add", "frontend/tracked.ts", "backend/tracked.go")
	run("commit", "-qm", "initial")
}

// call dispatches one capability with the fixture's agent cwd as the only live
// root, and returns the raw response plus the error.
func (fx gitFixture) call(t *testing.T, method string, params map[string]any) (string, error) {
	t.Helper()
	reg := registryWithCwds(t, fx.agentCwd)
	body, err := json.Marshal(params)
	if err != nil {
		t.Fatal(err)
	}
	res, err := reg.handle(context.Background(), method, json.RawMessage(body))
	return string(res), err
}

// mustRefuse asserts the ONE non-echoing refusal, and separately that nothing
// leaked. A handler that failed for an unrelated reason (a decode error, a
// missing param, ENOENT) proves nothing about confinement.
func (fx gitFixture) mustRefuse(t *testing.T, method string, params map[string]any) {
	t.Helper()
	res, err := fx.call(t, method, params)
	if err == nil {
		t.Fatalf("%s %v was ALLOWED — response: %s", method, params, truncate(res))
	}
	if !strings.Contains(err.Error(), refusalText) {
		t.Fatalf("%s %v was rejected for the wrong reason: %v", method, params, err)
	}
	if strings.Contains(res, secretMarker) || strings.Contains(err.Error(), secretMarker) {
		t.Fatalf("%s %v LEAKED the secret despite refusing: %s / %v", method, params, truncate(res), err)
	}
}

func truncate(s string) string {
	if len(s) > 400 {
		return s[:400] + "…"
	}
	return s
}

// ── the untracked leg: an arbitrary-file reader unless it is held to BOTH
//    the work-tree root and the ordinary workspace roots ──────────────────────

// The repository's own metadata directory, and credential files by name. Both
// gates live in pathIsSecret and both are reachable from a pathspec: `.git`
// because a config there is a program (filter.<drv>.clean is run by git add, and
// the namespaced exec keys are the ones no `-c` list can name), and the
// basenames because an agent cwd can be anywhere a token sits.
// ── the tracked leg: held to the repository, and the trade that leaves ───────

// BOTH root sets, independently. A second live agent makes its own repository an
// allowed root — so a path inside it satisfies the workspace-roots assertion —
// and it must STILL be refused, because it is outside the work-tree root the
// first call's git is running in. Without the first assertion this is a
// cross-project read.
// ── the cwd guard, on all four methods ──────────────────────────────────────

// A symlink that leaves the allowed root is resolved BEFORE the check, so the
// directory that was validated and the directory git runs in are the same one.
// BINDING DECISION 2 for the git legs: the canonical path the guard RETURNED is
// what git runs in, not the caller's string.
//
// The probe carries two defects at once. `<cwd>/sub/link/../nope/../` resolves,
// per component, to the agent cwd — a later ".." pops back onto ground that
// exists — while the raw string cannot be chdir'd into at all (`nope` is not
// there), and a textual Clean would name `<cwd>/sub` instead. So a handler that
// re-opens the caller's string fails, one that Cleans names the wrong directory,
// and only the guard's answer works.
func TestGitRunsInTheCanonicalCwdTheGuardReturned(t *testing.T) {
	fx := newGitFixture(t)
	for _, d := range []string{"sub", "real"} {
		if err := os.MkdirAll(filepath.Join(fx.agentCwd, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	gateSymlink(t, filepath.Join(fx.agentCwd, "real"), filepath.Join(fx.agentCwd, "sub", "link"))
	probe := filepath.Join(fx.agentCwd, "sub", "link") + "/../nope/../"

	res, err := fx.call(t, "git.status", map[string]any{"cwd": probe})
	if err != nil {
		t.Fatalf("git.status must run in the path the guard returned: %v", err)
	}
	var out gitStatusResult
	if err := json.Unmarshal([]byte(res), &out); err != nil {
		t.Fatal(err)
	}
	if out.Branch == nil {
		t.Fatalf("expected a branch from a real repository, got %s", truncate(res))
	}
}

// ── the floor: the guard must not simply deny everything ────────────────────

func TestGitReadsWorkInsideTheAllowedRoot(t *testing.T) {
	fx := newGitFixture(t)

	t.Run("status", func(t *testing.T) {
		res, err := fx.call(t, "git.status", map[string]any{"cwd": fx.agentCwd})
		if err != nil {
			t.Fatal(err)
		}
		var out gitStatusResult
		if err := json.Unmarshal([]byte(res), &out); err != nil {
			t.Fatal(err)
		}
		if out.Branch == nil || *out.Branch == "" {
			t.Errorf("expected a branch name, got %s", truncate(res))
		}
		// --untracked-files=all: every untracked file individually, so the
		// review pane never asks for an untracked diff of a directory.
		var found bool
		for _, f := range out.Files {
			if f.Path == "frontend/untracked.txt" && f.Staged == "?" {
				found = true
			}
		}
		if !found {
			t.Errorf("expected the untracked file in status, got %+v", out.Files)
		}
	})

	t.Run("log", func(t *testing.T) {
		res, err := fx.call(t, "git.log", map[string]any{"cwd": fx.agentCwd, "limit": 5})
		if err != nil {
			t.Fatal(err)
		}
		var out struct {
			Commits []gitLogEntry `json:"commits"`
		}
		if err := json.Unmarshal([]byte(res), &out); err != nil {
			t.Fatal(err)
		}
		if len(out.Commits) != 1 || out.Commits[0].Subject != "initial" || out.Commits[0].AuthoredAt == 0 {
			t.Fatalf("got %+v", out.Commits)
		}
	})

	t.Run("numstat", func(t *testing.T) {
		// Modify a tracked file so numstat has something to count.
		p := filepath.Join(fx.agentCwd, "tracked.ts")
		if err := os.WriteFile(p, []byte("export const a = 1;\nexport const b = 2;\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		res, err := fx.call(t, "git.numstat", map[string]any{"cwd": fx.agentCwd})
		if err != nil {
			t.Fatal(err)
		}
		var out struct {
			Files []gitNumstatEntry `json:"files"`
		}
		if err := json.Unmarshal([]byte(res), &out); err != nil {
			t.Fatal(err)
		}
		if len(out.Files) != 1 || out.Files[0].Path != "frontend/tracked.ts" ||
			out.Files[0].Added == nil || *out.Files[0].Added != 1 {
			t.Fatalf("got %+v", out.Files)
		}
	})

	t.Run("diff of a tracked change inside the agent cwd", func(t *testing.T) {
		p := filepath.Join(fx.agentCwd, "tracked.ts")
		if err := os.WriteFile(p, []byte("export const a = 2;\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		res, err := fx.call(t, "git.diff", map[string]any{"cwd": fx.agentCwd, "path": "frontend/tracked.ts"})
		if err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(res, "+export const a = 2;") {
			t.Fatalf("expected the change in the diff, got %s", truncate(res))
		}
	})
}

// A cwd that is allowed but is not a repository fails with git's own message,
// never with the containment refusal — the two mean different things to whoever
// is reading the Review pane's banner.
func TestGitOutsideAWorkTreeFailsWithItsOwnMessage(t *testing.T) {
	gateGit(t)
	dir, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	tempConfigHome(t)
	reg := registryWithCwds(t, dir)
	body, _ := json.Marshal(map[string]any{"cwd": dir})
	_, err = reg.handle(context.Background(), "git.status", json.RawMessage(body))
	if err == nil {
		t.Fatal("git.status on a non-repository must fail")
	}
	if strings.Contains(err.Error(), refusalText) {
		t.Fatalf("a non-repository must not read as a containment refusal: %v", err)
	}
	if !strings.Contains(err.Error(), "not inside a git work tree") {
		t.Fatalf("unexpected message: %v", err)
	}
}

// The full headless scope must retain the review actions. Catalog mode still
// leaves execution with its local desktop provider.
