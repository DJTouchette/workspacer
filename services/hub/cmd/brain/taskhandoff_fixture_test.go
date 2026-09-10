package main

import (
	"context"
	"encoding/json"
	"encoding/pem"
	"net/http/cgi"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
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
	handoffFixtureGit(t, remote, "init", "--bare", "--initial-branch=main")
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

// Production handlers and a temporary HTTPS Git server; only provider outcome
// is synthetic. Both orientations have independently owned on-disk custody.
func TestHandoffHTTPSRoundTripAndRestart(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("v1 explicitly refuses Windows ACL custody")
	}
	for _, orientation := range []string{"desktop-to-paired", "paired-to-desktop"} {
		t.Run(orientation, func(t *testing.T) {
			ctx := context.Background()
			source := t.TempDir()
			handoffFixtureGit(t, source, "init", "--initial-branch=main")
			if err := os.WriteFile(filepath.Join(source, "code.txt"), []byte("A\r\n"), 0600); err != nil {
				t.Fatal(err)
			}
			handoffFixtureGit(t, source, "add", "code.txt")
			handoffFixtureGit(t, source, "commit", "-m", "A")
			remote, ca := handoffFixtureRemote(t, source)
			handoffFixtureGit(t, source, "commit", "--allow-empty", "-m", "B is not remote HEAD")
			base := handoffFixtureGit(t, source, "rev-parse", "HEAD")
			if err := os.WriteFile(filepath.Join(source, ".git/info/exclude"), []byte(".workspacer/\n"), 0600); err != nil {
				t.Fatal(err)
			}
			reports := filepath.Join(source, ".workspacer/reports")
			if err := os.MkdirAll(reports, 0700); err != nil {
				t.Fatal(err)
			}
			input := []byte("# source evidence\r\n")
			if err := os.WriteFile(filepath.Join(reports, "brief.md"), input, 0600); err != nil {
				t.Fatal(err)
			}
			origin := &registry{handoffRoot: t.TempDir()}
			target := &registry{handoffRoot: t.TempDir()}
			binding := taskartifacts.RepositoryBinding{ID: "handoff-fixture-binding", Revision: "1", Repository: source, Remote: remote, RefPrefix: "refs/heads/handoff", Owner: "local-host", Origin: "handoff-fixture-origin", Export: true, Import: true, Cleanup: true, TLSCAFile: ca}
			fixtureWriteBindings(t, origin.handoffRoot, binding)
			binding.Owner = "synthetic-paired-owner"
			fixtureWriteBindings(t, target.handoffRoot, binding)
			task := strings.Repeat("a", 32)
			call := func(r *registry, owner, operation string, extra map[string]any) (handoffRecord, error) {
				p := map[string]any{"operation": operation, "binding": binding.ID, "task": task, "originKey": owner}
				for key, value := range extra {
					p[key] = value
				}
				raw, _ := json.Marshal(p)
				b, err := r.taskHandoff(ctx, raw)
				var rec handoffRecord
				if err == nil {
					err = json.Unmarshal(b, &rec)
				}
				return rec, err
			}
			must := func(r *registry, owner, operation string, extra map[string]any) handoffRecord {
				t.Helper()
				rec, err := call(r, owner, operation, extra)
				if err != nil {
					t.Fatalf("%s: %v", operation, err)
				}
				return rec
			}
			selections := []handoffSelection{{"brief.md", "report"}}
			outputs := []handoffSelection{{"result.png", "image"}}
			frozen := must(origin, "local-host", "freeze", map[string]any{"cwd": source, "provider": "claude", "selections": selections, "outputs": outputs})
			must(target, binding.Owner, "reserve", map[string]any{"plan": frozen.Plan})
			if _, err := call(target, "other-origin-owner", "status", nil); err == nil {
				t.Fatal("another origin read custody")
			}
			must(origin, "local-host", "publish", nil)
			if _, err := call(target, binding.Owner, "prepare", nil); err == nil {
				t.Fatal("missing required evidence admitted")
			}
			must(target, binding.Owner, "write", map[string]any{"direction": "input", "index": 0, "data": input})
			// Reconstructed registry reads the durable transfer receipt and bytes.
			target = &registry{handoffRoot: target.handoffRoot}
			prepared := must(target, binding.Owner, "prepare", nil)
			if handoffFixtureGit(t, prepared.Allocation, "rev-parse", "HEAD") != base {
				t.Fatal("used remote HEAD A instead of selected B")
			}
			if b, err := os.ReadFile(filepath.Join(prepared.Allocation, "code.txt")); err != nil || string(b) != "A\r\n" {
				t.Fatal("code CRLF changed")
			}
			must(target, binding.Owner, "prepare", nil)
			// Simulate a crash after worktree/artifact creation but before receipt
			// promotion. The same objects must resume without deleting anything.
			interrupted := prepared
			interrupted.State = "transferring"
			if err := saveHandoff(target.handoffDir(binding, task), &interrupted); err != nil {
				t.Fatal(err)
			}
			prepared = must(target, binding.Owner, "prepare", nil)
			if err := os.WriteFile(filepath.Join(prepared.Allocation, "code.txt"), []byte("C\r\n"), 0600); err != nil {
				t.Fatal(err)
			}
			handoffFixtureGit(t, prepared.Allocation, "add", "code.txt")
			handoffFixtureGit(t, prepared.Allocation, "commit", "-m", "C")
			target.remote = &remoteDispatchStore{m: map[string]*remoteDispatch{task: {lease: &dispatchLease{Owner: binding.Owner, Claimed: true}, last: &dispatchUpdate{Final: true}}}}
			if _, err := call(target, binding.Owner, "sealResult", nil); err == nil {
				t.Fatal("missing required result was sealed")
			}
			output := []byte{0x89, 'P', 'N', 'G', 0, 0xff, '\r', '\n'}
			if err := os.WriteFile(filepath.Join(prepared.Allocation, ".workspacer/handoffs", task, "result.png"), output, 0600); err != nil {
				t.Fatal(err)
			}
			sealed := must(target, binding.Owner, "sealResult", nil)
			must(origin, "local-host", "receiveResult", map[string]any{"manifest": sealed.Result})
			if _, err := call(origin, "local-host", "importResult", nil); err == nil {
				t.Fatal("custody without required bytes")
			}
			must(origin, "local-host", "write", map[string]any{"direction": "result", "index": 0, "data": output})
			origin = &registry{handoffRoot: origin.handoffRoot}
			received := must(origin, "local-host", "importResult", nil)
			must(origin, "local-host", "importResult", nil)
			if b, err := os.ReadFile(filepath.Join(received.Allocation, ".workspacer/handoffs", task, "result.png")); err != nil || string(b) != string(output) {
				t.Fatal("binary output custody mismatch")
			}
			if handoffFixtureGit(t, source, "rev-parse", "HEAD") != base {
				t.Fatal("active source checkout changed")
			}
			if _, err := call(target, binding.Owner, "cleanup", nil); err == nil {
				t.Fatal("cleanup before custody and acceptance")
			}
			must(target, binding.Owner, "custody", map[string]any{"digest": received.Custody})
			// Avoid timer side effects in this fixture; acceptance persists on disk.
			target.remote = nil
			must(target, binding.Owner, "disposition", map[string]any{"digest": received.Custody})
			if _, err := call(target, binding.Owner, "cleanup", nil); err == nil {
				t.Fatal("cleanup before grace")
			}
			target.remote = newRemoteDispatchStore()
			target.remote.m[task] = &remoteDispatch{sessionID: "fixture-worker", lease: &dispatchLease{Owner: binding.Owner, Claimed: true, Cwd: prepared.Allocation, Handoff: &handoffReceiptSelector{Binding: binding.ID, Digest: prepared.Digest}}, last: &dispatchUpdate{Final: true}}
			target.store = newSessionStore()
			target.store.set("fixture-worker", json.RawMessage(`{"sessionId":"fixture-worker","status":"ended"}`))
			accepted := must(target, binding.Owner, "status", nil)
			accepted.AcceptedAt = time.Now().Add(-handoffCleanupGrace - time.Hour).UnixMilli()
			if err := saveHandoff(target.handoffDir(binding, task), &accepted); err != nil {
				t.Fatal(err)
			}
			unexpected := filepath.Join(prepared.Allocation, "user-notes.txt")
			if err := os.WriteFile(unexpected, []byte("keep me"), 0600); err != nil {
				t.Fatal(err)
			}
			if _, err := call(target, binding.Owner, "cleanup", nil); err == nil {
				t.Fatal("cleanup removed unexpected user file")
			}
			if err := os.Remove(unexpected); err != nil {
				t.Fatal(err)
			}
			cleaned := must(target, binding.Owner, "cleanup", nil)
			if cleaned.State != "cleaned" {
				t.Fatal("accepted cleanup did not finish")
			}
			must(target, binding.Owner, "cleanup", nil)
			if _, err := os.Stat(prepared.Allocation); !os.IsNotExist(err) {
				t.Fatal("execution worktree not cleaned")
			}
			if b, err := os.ReadFile(filepath.Join(received.Allocation, ".workspacer/handoffs", task, "result.png")); err != nil || string(b) != string(output) {
				t.Fatal("cleanup lost receiver custody")
			}
		})
	}
}
