package main

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
)

func leaseRegistry(t *testing.T) (*registry, spawnParams) {
	t.Helper()
	s := newRemoteDispatchStore()
	if err := s.load(filepath.Join(t.TempDir(), "dispatches.json")); err != nil {
		t.Fatal(err)
	}
	id := "0123456789abcdef"
	s.m[id] = &remoteDispatch{dispatchID: id, lease: &dispatchLease{Owner: "origin-a", Repo: "remote-repo", Cwd: "isolated-remote-worktree", Provider: "claude", Expires: time.Now().Add(time.Minute).UnixMilli()}}
	r := &registry{remote: s, store: newSessionStore(), publish: func(string, json.RawMessage) {}}
	return r, spawnParams{RemoteOrigin: &remoteOriginParam{Protocol: bus.DispatchProtocol, DispatchID: id, OwnerKey: "origin-a"}, Cwd: "isolated-remote-worktree", Provider: "claude"}
}

func TestDispatchLeaseBindsOriginCwdProviderAndSingleAdmission(t *testing.T) {
	for _, mutation := range []string{"owner", "cwd", "provider", "expired"} {
		t.Run(mutation, func(t *testing.T) {
			r, p := leaseRegistry(t)
			switch mutation {
			case "owner":
				p.RemoteOrigin.OwnerKey = "different-origin"
			case "cwd":
				p.Cwd = "desktop-local-path"
			case "provider":
				p.Provider = "codex"
			case "expired":
				r.remote.m[p.RemoteOrigin.DispatchID].lease.Expires = 1
			}
			if r.claimDispatch(context.Background(), p) == nil {
				t.Fatal("mismatched remote admission was accepted")
			}
		})
	}
	r, p := leaseRegistry(t)
	if err := r.claimDispatch(context.Background(), p); err != nil {
		t.Fatal(err)
	}
	if r.claimDispatch(context.Background(), p) == nil {
		t.Fatal("same dispatch admitted twice")
	}
	restored := newRemoteDispatchStore()
	if err := restored.load(r.remote.file); err != nil {
		t.Fatal(err)
	}
	r.remote = restored
	if r.claimDispatch(context.Background(), p) == nil {
		t.Fatal("restart made a consumed admission reusable")
	}
}

func TestDispatchJournalFailureRefusesAdmissionAndPublication(t *testing.T) {
	r, p := leaseRegistry(t)
	r.remote.file = filepath.Join(t.TempDir(), "missing", "journal.json")
	if r.claimDispatch(context.Background(), p) == nil {
		t.Fatal("admitted without durable receipt")
	}
	if r.remote.record(p.RemoteOrigin.DispatchID, "worker") == nil {
		t.Fatal("recording succeeded without persistence")
	}
	published := false
	r.publish = func(string, json.RawMessage) { published = true }
	if r.emitDispatchUpdate(p.RemoteOrigin.DispatchID, dispatchKindFinished, "worker", fleetEntry{SessionID: "worker"}, true) || published {
		t.Fatal("published a terminal result without durable replay")
	}
}

func TestRemoteResultSurvivesRestartAndClosingTheCard(t *testing.T) {
	r, p := leaseRegistry(t)
	if err := r.remote.record(p.RemoteOrigin.DispatchID, "worker"); err != nil {
		t.Fatal(err)
	}
	for _, kind := range []string{dispatchKindProgress, dispatchKindBlocked, dispatchKindFinished} {
		if !r.emitDispatchUpdate(p.RemoteOrigin.DispatchID, kind, "worker", fleetEntry{SessionID: "worker", Label: "fixture"}, kind == dispatchKindFinished) {
			t.Fatal("update not retained")
		}
	}
	r.remote.forget("worker")
	restored := newRemoteDispatchStore()
	if err := restored.load(r.remote.file); err != nil {
		t.Fatal(err)
	}
	last, session, ok := restored.replay(p.RemoteOrigin.DispatchID)
	if !ok || session != "worker" || last == nil || !last.Final || last.Seq != 3 {
		t.Fatalf("lost terminal receipt: %+v", last)
	}
	if _, ok := restored.next(p.RemoteOrigin.DispatchID, false); ok {
		t.Fatal("terminal dispatch accepted later progress")
	}
}

func TestOldDispatchProtocolRefusesBeforeSpawn(t *testing.T) {
	r, p := leaseRegistry(t)
	p.RemoteOrigin.Protocol = 1
	if _, err := r.acceptRemoteOrigin(p); err == nil {
		t.Fatal("older protocol silently accepted")
	}
}

func TestDispatchWireUsesDesktopFieldNames(t *testing.T) {
	raw, err := json.Marshal(dispatchUpdate{Protocol: bus.DispatchProtocol, DispatchID: "0123456789abcdef", SessionID: "worker", Kind: dispatchKindProgress, Seq: 1, Entry: fleetEntry{Label: "Worker", SessionID: "worker", Note: "phase finished", NeedsDecision: true}})
	if err != nil {
		t.Fatal(err)
	}
	var data map[string]any
	if err := json.Unmarshal(raw, &data); err != nil {
		t.Fatal(err)
	}
	entry := data["entry"].(map[string]any)
	if entry["sessionId"] != "worker" || entry["label"] != "Worker" || entry["note"] != "phase finished" || entry["needsDecision"] != true {
		t.Fatalf("desktop cannot decode fleet entry: %s", raw)
	}
	if _, exists := entry["SessionID"]; exists {
		t.Fatal("internal Go field spelling leaked onto dispatch wire")
	}
}

func TestTerminalAcknowledgementIsBoundToOriginAndSurvivesRestart(t *testing.T) {
	r, p := leaseRegistry(t)
	if err := r.remote.record(p.RemoteOrigin.DispatchID, "worker"); err != nil {
		t.Fatal(err)
	}
	if !r.emitDispatchUpdate(p.RemoteOrigin.DispatchID, dispatchKindFinished, "worker", fleetEntry{SessionID: "worker"}, true) {
		t.Fatal("no terminal receipt")
	}
	call := func(owner string) (json.RawMessage, error) {
		raw, _ := json.Marshal(map[string]any{"dispatchId": p.RemoteOrigin.DispatchID, "originKey": owner, "ackedSeq": 1})
		return r.dispatchReplay(context.Background(), raw)
	}
	if _, err := call("other-origin"); err == nil {
		t.Fatal("different origin acknowledged a receipt")
	}
	if _, err := call("origin-a"); err != nil {
		t.Fatal(err)
	}
	restored := newRemoteDispatchStore()
	if err := restored.load(r.remote.file); err != nil {
		t.Fatal(err)
	}
	if restored.m[p.RemoteOrigin.DispatchID].acknowledgedAt == 0 {
		t.Fatal("acknowledgement was not durable")
	}
}

func TestRemoteFinishMissedDuringBrainRestartIsReconciledOnce(t *testing.T) {
	rig := newWakeRig(t)
	rig.reg.remote = newRemoteDispatchStore()
	const id = "0123456789abcdef"
	if err := rig.reg.remote.record(id, "remote-worker"); err != nil {
		t.Fatal(err)
	}
	published := 0
	rig.reg.publish = func(topic string, raw json.RawMessage) {
		if topic == bus.TopicDispatchUpdate {
			published++
		}
	}
	now := time.Now()
	rig.store.seed(map[string]json.RawMessage{"remote-worker": json.RawMessage(atTime("remote-worker", "/remote/repo", "input", now.Add(-fleetMissedWakeGrace-time.Minute)))})
	rig.d.setConv("remote-worker", dispatched("Recovered remote final result"))
	rig.fin.sweepMissedFinishes(context.Background(), now)
	rig.fin.sweepMissedFinishes(context.Background(), now)
	if published != 1 {
		t.Fatalf("missed remote finish produced %d callbacks, want one", published)
	}
	if len(rig.d.sent) != 0 {
		t.Fatal("remote result attempted to wake a peer-local manager")
	}
}
