package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func fixtureGit(t *testing.T, cwd string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = cwd
	cmd.Env = append(os.Environ(), "GIT_CONFIG_NOSYSTEM=1")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v: %s", args, err, out)
	}
	return strings.TrimSpace(string(out))
}

func TestHeadlessGitReviewActions(t *testing.T) {
	fx := newGitFixture(t)
	call := func(method string, p map[string]any) string {
		t.Helper()
		p["cwd"] = fx.agentCwd
		out, err := fx.call(t, method, p)
		if err != nil {
			t.Fatalf("%s: %v", method, err)
		}
		return out
	}
	if err := os.Remove(filepath.Join(fx.agentCwd, "tracked.ts")); err != nil {
		t.Fatal(err)
	}
	call("git.stage", map[string]any{})
	staged := fixtureGit(t, fx.repo, "diff", "--cached", "--name-status")
	if !strings.Contains(staged, "D\tfrontend/tracked.ts") || !strings.Contains(staged, "A\tfrontend/untracked.txt") || strings.Contains(staged, "backend/") {
		t.Fatalf("stage all escaped or lost additions/deletions: %s", staged)
	}
	call("git.unstage", map[string]any{"path": "frontend/untracked.txt"})
	if strings.Contains(fixtureGit(t, fx.repo, "diff", "--cached", "--name-only"), "untracked.txt") {
		t.Fatal("unstage failed")
	}
	call("git.stage", map[string]any{"path": "frontend/untracked.txt"})
	call("git.commit", map[string]any{"message": "review action"})
	hash := fixtureGit(t, fx.repo, "rev-parse", "HEAD")
	diff := call("git.commitDiff", map[string]any{"hash": hash, "path": "frontend/untracked.txt"})
	if !strings.Contains(diff, "+brand new line") {
		t.Fatalf("missing committed patch: %s", diff)
	}
	var stats struct {
		Files []gitNumstatEntry `json:"files"`
	}
	if err := json.Unmarshal([]byte(call("git.commitNumstat", map[string]any{"hash": hash})), &stats); err != nil || len(stats.Files) != 2 {
		t.Fatalf("commit stats: %+v / %v", stats, err)
	}
	remote := filepath.Join(fx.sandbox, "remote.git")
	fixtureGit(t, fx.repo, "init", "--bare", remote)
	fixtureGit(t, fx.repo, "remote", "add", "origin", remote)
	branch := fixtureGit(t, fx.repo, "branch", "--show-current")
	fixtureGit(t, fx.repo, "config", "branch."+branch+".remote", "origin")
	fixtureGit(t, fx.repo, "config", "branch."+branch+".merge", "refs/heads/"+branch)
	call("git.push", map[string]any{})
	if got := fixtureGit(t, remote, "rev-parse", "refs/heads/"+branch); got != hash {
		t.Fatalf("push: %s != %s", got, hash)
	}
}

func TestHeadlessGitMutationsRetainConfinement(t *testing.T) {
	fx := newGitFixture(t)
	for _, method := range []string{"git.stage", "git.unstage"} {
		for _, target := range []string{"backend/tracked.go", "../outside/secret.txt", filepath.Join(fx.repo, ".git", "config")} {
			fx.mustRefuse(t, method, map[string]any{"cwd": fx.agentCwd, "path": target})
		}
	}
	for _, method := range []string{"git.commitDiff", "git.commitNumstat"} {
		for _, hash := range []string{"--output=/tmp/escape", "HEAD", "HEAD~1", ""} {
			if _, err := fx.call(t, method, map[string]any{"cwd": fx.agentCwd, "hash": hash}); err == nil || !strings.Contains(err.Error(), "not a commit hash") {
				t.Fatalf("%s allowed non-hash %q: %v", method, hash, err)
			}
		}
	}
	if _, err := fx.call(t, "git.commit", map[string]any{"cwd": fx.agentCwd, "message": " "}); err == nil {
		t.Fatal("empty commit accepted")
	}
	if _, err := fx.call(t, "git.push", map[string]any{"cwd": fx.agentCwd}); err == nil {
		t.Fatal("push without destination reported success")
	}
}
