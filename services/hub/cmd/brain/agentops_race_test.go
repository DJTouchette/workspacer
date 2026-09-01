package main

import (
	"context"
	"encoding/json"
	"testing"
	"time"
)

// The response snapshot is deliberately detached from the armed watch. This
// is the deterministic half of the -race regression: sweepThresholds is free
// to bind telemetry/state after notifyWhen returns, but no response marshal can
// retain the map-owned pointer or any of its mutable pointees.
func TestThresholdWatchResponseSnapshotIsDetachedFromSweepState(t *testing.T) {
	tokens := 10.0
	pct := 80.0
	epoch := telemetryEpoch("1788888888888888901")
	live := &thresholdWatch{
		ID:             "w1",
		Tokens:         &tokens,
		ContextUsedPct: &pct,
		ContextEpoch:   &epoch,
		State:          "waitingForTelemetry",
	}
	snapshot := snapshotThresholdWatch(live)

	// This is exactly the mutation a sweep performs while the watch stays
	// armed. A response produced from the snapshot remains frozen.
	*live.Tokens = 99
	*live.ContextUsedPct = 55
	next := telemetryEpoch("1788888888888888902")
	live.ContextEpoch = &next
	live.State = "armed"

	if got := *snapshot.Tokens; got != 10 {
		t.Fatalf("snapshot tokens changed with live watch: %v", got)
	}
	if got := *snapshot.ContextUsedPct; got != 80 {
		t.Fatalf("snapshot context threshold changed with live watch: %v", got)
	}
	if got := *snapshot.ContextEpoch; got != epoch {
		t.Fatalf("snapshot epoch changed with live watch: %q", got)
	}
	if snapshot.State != "waitingForTelemetry" {
		t.Fatalf("snapshot state changed with live watch: %q", snapshot.State)
	}
}

// This is the call-site half of the regression. The hook forces the exact
// interleaving that made the old Unlock-then-jsonResult(w) racy: a real sweep
// binds the live watch after notifyWhen releases watchMu but before its response
// is encoded. The caller must still receive the state that was true at arm time.
func TestNotifyWhenResponseIsDetachedBeforeConcurrentSweep(t *testing.T) {
	now := time.Now().UTC()
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := fleetReg(t, srv.URL,
		map[string]string{
			"mgr":    row("mgr", "/w", "input"),
			"worker": `{"session_id":"worker","cwd":"/w/p","mode":"responding","provider":"codex"}`,
		},
		map[string]spawnMeta{"worker": {ParentSessionID: "mgr"}})
	reg.watchResponseSnapshotReady = func() {
		reg.store.updateStatusLine("worker", contextHealthStatus(
			t, now, 100_000, 200_000, telemetryEpoch("1788888888888888901"), "codex", "runtime",
		))
		reg.sweepThresholds(context.Background(), now)
	}

	result, err := reg.notifyWhen(context.Background(), json.RawMessage(
		`{"sessionId":"worker","contextUsedPct":80}`,
	))
	if err != nil {
		t.Fatal(err)
	}
	var response thresholdWatch
	if err := json.Unmarshal(result, &response); err != nil {
		t.Fatal(err)
	}
	if response.State != "waitingForTelemetry" || response.ContextEpoch != nil {
		t.Fatalf("response observed sweep mutation instead of arm-time snapshot: %+v", response)
	}

	reg.watchMu.Lock()
	live := reg.watches[response.ID]
	reg.watchMu.Unlock()
	if live == nil || live.ContextEpoch == nil || *live.ContextEpoch != telemetryEpoch("1788888888888888901") {
		t.Fatalf("deterministic sweep did not bind the live watch: %+v", live)
	}
}
