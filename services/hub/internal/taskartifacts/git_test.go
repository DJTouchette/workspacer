package taskartifacts

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestCheckpointPreflightDoesNotExecuteRepositoryFilters(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX fixture; Windows custody is refused separately")
	}
	repo := t.TempDir()
	git := func(args ...string) string {
		t.Helper()
		cmd := exec.Command("git", append([]string{"-c", "user.name=Fixture", "-c", "user.email=fixture@example.test"}, args...)...)
		cmd.Dir = repo
		b, err := cmd.CombinedOutput()
		if err != nil {
			t.Fatalf("fixture git: %v: %s", err, b)
		}
		return strings.TrimSpace(string(b))
	}
	git("init")
	if err := os.WriteFile(filepath.Join(repo, "file.txt"), []byte("ordinary code\r\n"), 0600); err != nil {
		t.Fatal(err)
	}
	git("add", "file.txt")
	git("commit", "-m", "fixture")
	ctx := context.Background()
	commit, _, err := CheckSource(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	if err := VerifyTree(ctx, repo, commit); err != nil {
		t.Fatal(err)
	}
	child := filepath.Join(repo, "subdirectory")
	if err := os.Mkdir(child, 0700); err != nil {
		t.Fatal(err)
	}
	if _, _, err := CheckSource(ctx, child); err == nil {
		t.Fatal("subdirectory mapping exported its parent repository")
	}
	alias := filepath.Join(t.TempDir(), "alias")
	if err := os.Symlink(repo, alias); err != nil {
		t.Fatal(err)
	}
	if _, _, err := CheckSource(ctx, alias); err == nil {
		t.Fatal("mutable source alias admitted")
	}
	marker := filepath.Join(repo, "filter-ran")
	report := ".workspacer/reports/scout.md"
	if err := os.MkdirAll(filepath.Dir(filepath.Join(repo, report)), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(repo, report), []byte("selected report"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, _, err := CheckSource(ctx, repo); err == nil {
		t.Fatal("unselected report hidden")
	}
	if _, _, err := CheckSource(ctx, repo, report); err != nil {
		t.Fatal("explicit selected report requires no ignore configuration", err)
	}
	git("add", report)
	git("commit", "-m", "tracked report fixture")
	if err := os.WriteFile(filepath.Join(repo, report), []byte("dirty tracked report"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, _, err := CheckSource(ctx, repo, report); err == nil {
		t.Fatal("selection hid tracked WIP")
	}
	git("checkout", "--", report)
	git("config", "filter.untrusted.clean", "touch "+marker)
	if err := os.WriteFile(filepath.Join(repo, ".git/info/attributes"), []byte("* filter=untrusted\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, _, err := CheckSource(ctx, repo); err == nil {
		t.Fatal("execution-valued repository config admitted")
	}
	if _, err := os.Stat(marker); !os.IsNotExist(err) {
		t.Fatal("repository filter executed during preflight")
	}
	git("config", "--remove-section", "filter.untrusted")
	git("config", "url.https://unapproved.example.invalid/.insteadOf", "https://approved.example.invalid/")
	if _, _, err := CheckSource(ctx, repo); err == nil {
		t.Fatal("remote rewrite admitted")
	}
}
