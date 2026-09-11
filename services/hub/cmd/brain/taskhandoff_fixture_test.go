package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"net/http/cgi"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/taskartifacts"
)

func handoffFixtureGit(t *testing.T, cwd string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-c", "user.name=Workspace fixture", "-c", "user.email=fixture@example.test", "-c", "core.hooksPath=" + os.DevNull}, args...)...)
	cmd.Dir = cwd
	b, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("fixture Git: %v: %s", err, b)
	}
	return strings.TrimSpace(string(b))
}

// A private HTTPS Git remote, reachable only within this hosted test process.
// No personal repository, live pairing or deployment credential is used.
func handoffFixtureRemote(t *testing.T, seed string) (string, string) {
	t.Helper()
	root := t.TempDir()
	remote := filepath.Join(root, "approved.git")
	if err := os.Mkdir(remote, 0700); err != nil {
		t.Fatal(err)
	}
	format := handoffFixtureGit(t, seed, "rev-parse", "--show-object-format")
	handoffFixtureGit(t, remote, "init", "--bare", "--initial-branch=main", "--object-format="+format)
	handoffFixtureGit(t, remote, "config", "http.receivepack", "true")
	handoffFixtureGit(t, seed, "push", remote, "HEAD:refs/heads/main")
	git, err := exec.LookPath("git")
	if err != nil {
		t.Fatal(err)
	}
	server := httptest.NewTLSServer(&cgi.Handler{Path: git, Args: []string{"http-backend"}, Env: []string{"GIT_PROJECT_ROOT=" + root, "GIT_HTTP_EXPORT_ALL=1", "GIT_CONFIG_NOSYSTEM=1", "GIT_CONFIG_GLOBAL=" + os.DevNull}})
	t.Cleanup(server.Close)
	ca := filepath.Join(root, "fixture-ca.pem")
	if err := os.WriteFile(ca, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: server.Certificate().Raw}), 0600); err != nil {
		t.Fatal(err)
	}
	return server.URL + "/approved.git", ca
}

func fixtureWriteBindings(t *testing.T, root string, bindings ...taskartifacts.RepositoryBinding) {
	t.Helper()
	if err := os.MkdirAll(root, 0700); err != nil {
		t.Fatal(err)
	}
	b, err := json.Marshal(bindings)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "handoff-bindings.json"), b, 0600); err != nil {
		t.Fatal(err)
	}
}

func configureHandoffChainFixture(t *testing.T, receiver *registry, seed string) map[string]string {
	originBus := os.Getenv("WKS_HANDOFF_ORIGIN_BUS")
	if originBus == "" {
		return nil
	}
	source := os.Getenv("WKS_HANDOFF_SOURCE")
	originRoot := os.Getenv("WKS_HANDOFF_ORIGIN_CONFIG")
	remote, ca := handoffFixtureRemote(t, seed)
	handoffFixtureGit(t, source, "commit", "--allow-empty", "-m", "selected checkpoint B")
	commit := handoffFixtureGit(t, source, "rev-parse", "HEAD")
	if err := os.WriteFile(filepath.Join(source, ".git", "info", "exclude"), []byte(".workspacer/\n"), 0600); err != nil {
		t.Fatal(err)
	}
	reports := filepath.Join(source, ".workspacer", "reports")
	if err := os.MkdirAll(reports, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(reports, "scout.md"), []byte("# Task evidence\nImplement at the selected checkpoint.\n"), 0600); err != nil {
		t.Fatal(err)
	}
	png, err := base64.StdEncoding.DecodeString("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1cAAAAASUVORK5CYII=")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(reports, "diagram.png"), png, 0600); err != nil {
		t.Fatal(err)
	}
	binding := taskartifacts.RepositoryBinding{ID: "workspace-fixture-binding", Revision: "1", Repository: source, Remote: remote, RefPrefix: "refs/heads/wks-transfer", Owner: "local-host", Origin: "workspace-fixture-origin", Export: true, Import: true, Cleanup: true, TLSCAFile: ca}
	fixtureWriteBindings(t, originRoot, binding)
	binding.Repository, binding.Owner = seed, bus.TokenFingerprint("paired-fixture-operator")
	fixtureWriteBindings(t, receiver.handoffConfigDir(), binding)
	origin := &registry{handoffRoot: originRoot}
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	client := newBusClient(originBus, "dispatch-chain-synthetic-host", []string{"agents.taskHandoff"}, origin.handle)
	go client.run(ctx)
	return map[string]string{"handoffBinding": binding.ID, "sourceCommit": commit}
}

func TestHandoffCleanupRequiresCustodyAcceptanceAndGrace(t *testing.T) {
	now := time.Unix(1000000, 0)
	m := taskartifacts.Manifest{Version: 1, Task: strings.Repeat("a", 32), Origin: strings.Repeat("b", 32), Producer: strings.Repeat("a", 32), Commit: strings.Repeat("c", 40), ObjectFormat: "sha1", Entries: []taskartifacts.Entry{}}
	digest, err := m.Seal()
	if err != nil {
		t.Fatal(err)
	}
	binding := taskartifacts.RepositoryBinding{Cleanup: true}
	rec := &handoffRecord{State: "result-sealed", Result: &m, Custody: digest, AcceptedAt: now.Add(-handoffCleanupGrace).UnixMilli()}
	if err := handoffCleanupAllowed(binding, rec, now); err != nil {
		t.Fatal(err)
	}
	for _, mutate := range []func(*handoffRecord){func(r *handoffRecord) { r.Custody = "" }, func(r *handoffRecord) { r.AcceptedAt = 0 }, func(r *handoffRecord) { r.Keep = true }, func(r *handoffRecord) { r.State = "needs-checkpoint" }, func(r *handoffRecord) { r.AcceptedAt++ }} {
		copy := *rec
		mutate(&copy)
		if handoffCleanupAllowed(binding, &copy, now) == nil {
			t.Fatal("unsafe cleanup admitted")
		}
	}
}

func TestHandoffGitRemoteExactCheckpoint(t *testing.T) {
	for _, format := range []string{"sha1", "sha256"} {
		t.Run(format, func(t *testing.T) {
			ctx := context.Background()
			sourceRoot := t.TempDir()
			handoffFixtureGit(t, sourceRoot, "init", "--initial-branch=main", "--object-format="+format)
			if err := os.WriteFile(filepath.Join(sourceRoot, "marker.txt"), []byte("A\n"), 0600); err != nil {
				t.Fatal(err)
			}
			handoffFixtureGit(t, sourceRoot, "add", "marker.txt")
			handoffFixtureGit(t, sourceRoot, "commit", "-m", "remote checkpoint A")
			base := handoffFixtureGit(t, sourceRoot, "rev-parse", "HEAD")
			remote, ca := handoffFixtureRemote(t, sourceRoot)
			source := filepath.Join(t.TempDir(), "selected-source")
			handoffFixtureGit(t, sourceRoot, "worktree", "add", "-b", "Task-Source", source, "HEAD")
			if err := os.WriteFile(filepath.Join(source, "marker.txt"), []byte("B\n"), 0600); err != nil {
				t.Fatal(err)
			}
			handoffFixtureGit(t, source, "add", "marker.txt")
			handoffFixtureGit(t, source, "commit", "-m", "selected checkpoint B")
			commit := handoffFixtureGit(t, source, "rev-parse", "HEAD")
			reports := filepath.Join(source, ".workspacer", "reports")
			if err := os.MkdirAll(reports, 0700); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(filepath.Join(reports, "brief.md"), []byte("scout bytes\r\n"), 0600); err != nil {
				t.Fatal(err)
			}
			indexPath := handoffFixtureGit(t, source, "rev-parse", "--path-format=absolute", "--git-path", "index")
			beforeIndex, err := os.ReadFile(indexPath)
			if err != nil {
				t.Fatal(err)
			}
			origin, receiver := &registry{handoffRoot: t.TempDir()}, &registry{handoffRoot: t.TempDir()}
			binding := taskartifacts.RepositoryBinding{ID: "fixture-repository-binding", Revision: "1", Repository: sourceRoot, Remote: remote, RefPrefix: "refs/heads/wks-transfer", Owner: "receiver-owner", Origin: "fixture-desktop-origin", Export: true, Import: true, Cleanup: true, TLSCAFile: ca}
			fixtureWriteBindings(t, origin.handoffRoot, binding)
			fixtureWriteBindings(t, receiver.handoffRoot, binding)
			call := func(r *registry, p handoffRequest) (handoffRecord, error) {
				raw, err := json.Marshal(p)
				if err != nil {
					t.Fatal(err)
				}
				answer, err := r.handle(ctx, "agents.taskHandoff", raw)
				if err != nil {
					return handoffRecord{}, err
				}
				var rec handoffRecord
				if err := json.Unmarshal(answer, &rec); err != nil {
					t.Fatal(err)
				}
				return rec, nil
			}
			prepare := func(task string, move bool) (handoffRecord, error) {
				frozen, err := call(origin, handoffRequest{Operation: "freeze", OriginKey: "local-host", Binding: binding.ID, Task: task, Provider: "synthetic", Cwd: source, Selections: []handoffSelection{{Name: "brief.md", Kind: "report"}}})
				if err != nil {
					t.Fatal(err)
				}
				if frozen.Plan.Input.Commit != commit || frozen.Plan.Input.ObjectFormat != format {
					t.Fatal("source checkpoint identity lost")
				}
				if _, err := call(receiver, handoffRequest{Operation: "reserve", OriginKey: binding.Owner, Binding: binding.ID, Task: task, Plan: &frozen.Plan}); err != nil {
					t.Fatal(err)
				}
				if _, err := call(receiver, handoffRequest{Operation: "prepare", OriginKey: binding.Owner, Binding: binding.ID, Task: task}); err == nil {
					t.Fatal("required bytes missing but prepared")
				}
				if _, err := call(origin, handoffRequest{Operation: "publish", OriginKey: "local-host", Binding: binding.ID, Task: task}); err != nil {
					t.Fatal(err)
				}
				if _, err := call(receiver, handoffRequest{Operation: "write", OriginKey: binding.Owner, Binding: binding.ID, Task: task, Direction: "input", Data: []byte("scout bytes\r\n")}); err != nil {
					t.Fatal(err)
				}
				if move {
					ref, _ := binding.Ref(task, "input")
					if _, err := binding.GitRemote(ctx, sourceRoot, "push", "--force-with-lease="+ref+":"+commit, "--", remote, base+":"+ref); err != nil {
						t.Fatal(err)
					}
				}
				return call(receiver, handoffRequest{Operation: "prepare", OriginKey: binding.Owner, Binding: binding.ID, Task: task})
			}
			task := strings.Repeat("a", 32)
			prepared, err := prepare(task, false)
			if err != nil {
				t.Fatal(err)
			}
			if got := handoffFixtureGit(t, prepared.Allocation, "rev-parse", "HEAD"); got != commit {
				t.Fatal("receiver used HEAD A instead of selected B")
			}
			copied, err := os.ReadFile(filepath.Join(prepared.Allocation, ".workspacer", "handoffs", task, "brief.md"))
			if err != nil || string(copied) != "scout bytes\r\n" {
				t.Fatal("artifact bytes changed", err)
			}
			if _, err := receiver.preparedHandoff(ctx, binding.Owner, task, &handoffReceiptSelector{Version: 1, Binding: binding.ID, Digest: prepared.Digest, AllocationId: prepared.AllocationId}); err != nil {
				t.Fatal(err)
			}
			restarted := &registry{handoffRoot: receiver.handoffRoot}
			if _, err := call(restarted, handoffRequest{Operation: "prepare", OriginKey: binding.Owner, Binding: binding.ID, Task: task}); err != nil {
				t.Fatal("prepared receipt did not survive restart", err)
			}
			if _, err := prepare(strings.Repeat("b", 32), true); err == nil {
				t.Fatal("moved input ref admitted")
			}
			afterIndex, err := os.ReadFile(indexPath)
			if err != nil || string(beforeIndex) != string(afterIndex) {
				t.Fatal("source index changed")
			}
			if handoffFixtureGit(t, sourceRoot, "rev-parse", "HEAD") != base || handoffFixtureGit(t, source, "symbolic-ref", "--short", "HEAD") != "Task-Source" {
				t.Fatal("user branches changed")
			}
		})
	}
}
