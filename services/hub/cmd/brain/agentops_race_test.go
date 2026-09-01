package main

import "testing"

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
