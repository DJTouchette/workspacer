package main

import (
	"context"
	"encoding/json"
	"github.com/djtouchette/workspacer-hub/internal/taskartifacts"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// GitHub's private workflow artifact channel carries the frozen RPC envelope
// and fixture Git server state between distinct OS runners. Each phase runs
// the production handlers, HTTPS Git transport, native filesystem and ACLs.
// No live workspace or deployment participates. The Linux job is the synthetic
// provider; source/return/continuation execute on native Windows runners.
func TestCrossPlatformHandoffStage(t *testing.T) {
	stage := os.Getenv("WKS_HANDOFF_STAGE")
	if stage == "" {
		t.Skip("staged hosted cross-OS fixture")
	}
	if stage == "worker" && runtime.GOOS != "linux" || stage != "worker" && runtime.GOOS != "windows" {
		t.Fatal("fixture OS contract not met")
	}
	transport := os.Getenv("WKS_HANDOFF_TRANSPORT")
	if !filepath.IsAbs(transport) {
		t.Fatal("absolute fixture transport required")
	}
	if err := os.MkdirAll(transport, 0700); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	source := t.TempDir()
	handoffFixtureGit(t, source, "init", "--initial-branch=main")
	if stage == "source" {
		if err := os.WriteFile(filepath.Join(source, "code.txt"), []byte("checkpoint A\r\n"), 0600); err != nil {
			t.Fatal(err)
		}
		handoffFixtureGit(t, source, "add", "code.txt")
		handoffFixtureGit(t, source, "commit", "-m", "A")
	}
	remote, ca := handoffFixtureRemoteAt(t, filepath.Join(transport, "git-server"), source)
	binding := taskartifacts.RepositoryBinding{ID: "cross-platform-fixture", Revision: "1", Repository: source, Remote: remote, RefPrefix: "refs/heads/fixture-transfer", Owner: "local-host", Origin: "windows-fixture-origin", Import: true, Export: true, Cleanup: true, TLSCAFile: ca}
	reg := &registry{handoffRoot: t.TempDir()}
	task := strings.Repeat("c", 64)
	owner := "local-host"
	if stage == "worker" {
		owner = "windows-origin-fixture-token"
		binding.Owner = owner
	}
	fixtureWriteBindings(t, reg.handoffRoot, binding)
	call := func(operation string, extra map[string]any) handoffRecord {
		t.Helper()
		p := map[string]any{"operation": operation, "binding": binding.ID, "task": task, "originKey": owner}
		for k, v := range extra {
			p[k] = v
		}
		raw, _ := json.Marshal(p)
		out, err := reg.taskHandoff(ctx, raw)
		if err != nil {
			t.Fatalf("%s/%s: %v", stage, operation, err)
		}
		var rec handoffRecord
		if err := json.Unmarshal(out, &rec); err != nil {
			t.Fatal(err)
		}
		return rec
	}
	type envelope struct {
		Frozen handoffRecord
		Result *taskartifacts.Manifest
		Input  []byte
		Output []byte
	}
	var wire envelope
	envelopePath := filepath.Join(transport, "rpc-envelope.json")
	if stage != "source" {
		raw, err := os.ReadFile(envelopePath)
		if err != nil || json.Unmarshal(raw, &wire) != nil {
			t.Fatal("invalid transported envelope", err)
		}
	}
	switch stage {
	case "source":
		handoffFixtureGit(t, source, "commit", "--allow-empty", "-m", "intended B, remote remains A")
		reports := filepath.Join(source, ".workspacer/reports")
		if err := os.MkdirAll(reports, 0700); err != nil {
			t.Fatal(err)
		}
		wire.Input = []byte("# Windows scout input\r\n")
		wire.Output = []byte("# Linux scout result\r\n")
		if err := os.WriteFile(filepath.Join(reports, "scout.md"), wire.Input, 0600); err != nil {
			t.Fatal(err)
		}
		wire.Frozen = call("freeze", map[string]any{"cwd": source, "provider": "claude", "selections": []handoffSelection{{"scout.md", "report"}}, "outputs": []handoffSelection{{"implementation.md", "report"}}})
		call("publish", nil)
		if err := taskartifacts.VerifyPrivateTree(reg.handoffDir(binding, task)); err != nil {
			t.Fatal(err)
		}
	case "worker":
		call("reserve", map[string]any{"plan": wire.Frozen.Plan})
		call("write", map[string]any{"direction": "input", "data": wire.Input})
		prepared := call("prepare", nil)
		if handoffFixtureGit(t, prepared.Allocation, "rev-parse", "HEAD") != wire.Frozen.Plan.Input.Commit {
			t.Fatal("Linux worker chose A instead of Windows B")
		}
		evidence := filepath.Join(prepared.Allocation, ".workspacer/handoffs", task, "scout.md")
		b, err := os.ReadFile(evidence)
		if err != nil || string(b) != string(wire.Input) {
			t.Fatal("Windows bytes changed on Linux")
		}
		if err := os.WriteFile(filepath.Join(prepared.Allocation, "code.txt"), []byte("Linux result C\r\n"), 0600); err != nil {
			t.Fatal(err)
		}
		handoffFixtureGit(t, prepared.Allocation, "add", "code.txt")
		handoffFixtureGit(t, prepared.Allocation, "commit", "-m", "C")
		if err := os.WriteFile(filepath.Join(filepath.Dir(evidence), "implementation.md"), wire.Output, 0600); err != nil {
			t.Fatal(err)
		}
		reg.remote = &remoteDispatchStore{m: map[string]*remoteDispatch{task: {lease: &dispatchLease{Owner: owner, Claimed: true}, last: &dispatchUpdate{Final: true}}}}
		wire.Result = call("sealResult", nil).Result
	case "receive":
		ref, _ := binding.Ref(task, "input")
		if _, err := binding.GitRemote(ctx, source, "fetch", remote, ref); err != nil {
			t.Fatal(err)
		}
		handoffFixtureGit(t, source, "checkout", "-B", "main", wire.Frozen.Plan.Input.Commit)
		dir := reg.handoffDir(binding, task)
		if err := taskartifacts.MakePrivateDirectory(dir); err != nil {
			t.Fatal(err)
		}
		if err := saveHandoff(dir, &wire.Frozen); err != nil {
			t.Fatal(err)
		}
		call("receiveResult", map[string]any{"manifest": wire.Result})
		call("write", map[string]any{"direction": "result", "data": wire.Output})
		received := call("importResult", nil)
		if received.Custody == "" || received.AllocationId == "" || handoffFixtureGit(t, received.Allocation, "rev-parse", "HEAD") != wire.Result.Commit {
			t.Fatal("Windows exact result custody missing")
		}
		if err := taskartifacts.VerifyPrivateTree(received.Allocation); err != nil {
			t.Fatal(err)
		}
		b, err := os.ReadFile(filepath.Join(received.Allocation, ".workspacer/handoffs", task, "implementation.md"))
		if err != nil || string(b) != string(wire.Output) {
			t.Fatal("Linux report bytes changed on Windows")
		}
		predecessor := task
		task = strings.Repeat("d", 64)
		next := call("freeze", map[string]any{"cwd": source, "fromTask": predecessor, "provider": "codex", "selections": []handoffSelection{{"implementation.md", "report"}}})
		local := call("prepareLocal", nil)
		call("claimLocal", map[string]any{"digest": next.Digest})
		if handoffFixtureGit(t, local.Allocation, "rev-parse", "HEAD") != wire.Result.Commit {
			t.Fatal("Windows local implementation lost remote checkpoint")
		}
		if err := taskartifacts.VerifyPrivateTree(local.Allocation); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(local.Allocation, "local.txt"), []byte("Windows local implementation\r\n"), 0600); err != nil {
			t.Fatal(err)
		}
		handoffFixtureGit(t, local.Allocation, "add", "local.txt")
		handoffFixtureGit(t, local.Allocation, "commit", "-m", "D Windows implementation")
		if _, _, err := taskartifacts.CheckSource(ctx, local.Allocation); err != nil {
			t.Fatal(err)
		}
		if handoffFixtureGit(t, source, "rev-parse", "HEAD") != wire.Frozen.Plan.Input.Commit {
			t.Fatal("Windows active checkout changed")
		}
	default:
		t.Fatal("unknown fixture stage")
	}
	raw, _ := json.Marshal(wire)
	if err := os.WriteFile(envelopePath, raw, 0600); err != nil {
		t.Fatal(err)
	}
	t.Logf("verified hosted stage=%s nativeOS=%s source=%s", stage, runtime.GOOS, wire.Frozen.Plan.Input.Commit)
}
