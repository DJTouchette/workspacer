package main

import (
	"context"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"runtime"
	"sync"
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

// This source contract binds notifyWhen's real return path to the detacher.
// It deliberately checks Go structure and data flow, rather than a line or
// spelling: the snapshot of the map-owned watch has to happen before unlock,
// and jsonResult has to receive that same snapshot afterwards. In particular,
// restoring `Unlock(); return jsonResult(w)` fails even if a stress schedule
// happens not to expose the race on a particular machine.
func TestNotifyWhenSnapshotsTheLiveWatchBeforeUnlocking(t *testing.T) {
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, "agentops.go", nil, 0)
	if err != nil {
		t.Fatal(err)
	}

	var notifyWhen *ast.FuncDecl
	for _, decl := range file.Decls {
		fn, ok := decl.(*ast.FuncDecl)
		if ok && fn.Name.Name == "notifyWhen" {
			notifyWhen = fn
			break
		}
	}
	if notifyWhen == nil {
		t.Fatal("notifyWhen is missing; the notify/sweep response boundary moved")
	}

	var snapshotName string
	var snapshotPos token.Pos
	ast.Inspect(notifyWhen.Body, func(node ast.Node) bool {
		var names []*ast.Ident
		var values []ast.Expr
		switch n := node.(type) {
		case *ast.AssignStmt:
			if len(n.Lhs) != len(n.Rhs) {
				return true
			}
			values = n.Rhs
			for _, lhs := range n.Lhs {
				name, ok := lhs.(*ast.Ident)
				if !ok {
					return true
				}
				names = append(names, name)
			}
		case *ast.ValueSpec:
			if len(n.Names) != len(n.Values) {
				return true
			}
			names, values = n.Names, n.Values
		default:
			return true
		}
		for i, value := range values {
			call, ok := value.(*ast.CallExpr)
			if !ok {
				continue
			}
			callee, ok := call.Fun.(*ast.Ident)
			if ok && callee.Name == "snapshotThresholdWatch" {
				snapshotName = names[i].Name
				snapshotPos = call.Pos()
			}
		}
		return true
	})
	if snapshotName == "" {
		t.Fatal("notifyWhen no longer snapshots its map-owned watch before returning")
	}

	var unlockPos token.Pos
	var marshalPos token.Pos
	ast.Inspect(notifyWhen.Body, func(node ast.Node) bool {
		call, ok := node.(*ast.CallExpr)
		if !ok {
			return true
		}
		if selector, ok := call.Fun.(*ast.SelectorExpr); ok && selector.Sel.Name == "Unlock" && call.Pos() > snapshotPos {
			if unlockPos == token.NoPos || call.Pos() < unlockPos {
				unlockPos = call.Pos()
			}
		}
		callee, ok := call.Fun.(*ast.Ident)
		if !ok || callee.Name != "jsonResult" || len(call.Args) != 1 {
			return true
		}
		result, ok := call.Args[0].(*ast.Ident)
		if ok && result.Name == snapshotName {
			marshalPos = call.Pos()
		}
		return true
	})
	if unlockPos == token.NoPos || marshalPos == token.NoPos || !(snapshotPos < unlockPos && unlockPos < marshalPos) {
		t.Fatalf("notifyWhen must snapshot under watchMu before unlock and pass that snapshot to jsonResult (snapshot=%d unlock=%d marshal=%d)", snapshotPos, unlockPos, marshalPos)
	}
}

// This is the live-path half of the regression. Each invocation races a real
// notifyWhen call against the real sweepThresholds mutation, without a hook or
// test-only production seam. The structural contract above makes the bad live
// pointer return deterministically red; this stress test keeps the actual
// lock, snapshot, JSON and sweep paths race-clean under -race.
func TestNotifyWhenAndSweepThresholdsConcurrentRaceFree(t *testing.T) {
	const attempts = 64
	now := time.Now().UTC()
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()

	start := make(chan struct{})
	errs := make(chan error, attempts)
	var wg sync.WaitGroup
	for i := 0; i < attempts; i++ {
		reg := fleetReg(t, srv.URL,
			map[string]string{
				"mgr":    row("mgr", "/w", "input"),
				"worker": `{"session_id":"worker","cwd":"/w/p","mode":"responding","provider":"codex"}`,
			},
			map[string]spawnMeta{"worker": {ParentSessionID: "mgr"}})
		status := contextHealthStatus(t, now, 200_000, 200_000, telemetryEpoch("1788888888888888901"), "codex", "runtime")

		wg.Add(2)
		go func() {
			defer wg.Done()
			<-start
			result, err := reg.notifyWhen(context.Background(), json.RawMessage(
				`{"sessionId":"worker","contextUsedPct":100}`,
			))
			if err != nil {
				errs <- err
				return
			}
			var response thresholdWatch
			if err := json.Unmarshal(result, &response); err != nil {
				errs <- err
				return
			}
			if response.ID == "" {
				errs <- fmt.Errorf("notifyWhen returned no watch id")
			}
		}()
		go func() {
			defer wg.Done()
			<-start
			// The first sweep can legitimately arrive before notifyWhen arms its
			// watch. Repeating gives the live call and the lock-protected map a
			// real concurrent interleaving without controlling either production
			// function's schedule.
			for sweep := 0; sweep < 64; sweep++ {
				reg.store.updateStatusLine("worker", status)
				reg.sweepThresholds(context.Background(), now)
				runtime.Gosched()
			}
		}()
	}
	close(start)
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Error(err)
	}
}
