package main

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/flyapi"
	"github.com/djtouchette/workspacer-hub/internal/quiescence"
)

type fakeMachinePower struct {
	checkErr error
	stops    atomic.Int32
	done     chan struct{}
}

func (f *fakeMachinePower) Check(context.Context) error { return f.checkErr }
func (f *fakeMachinePower) Stop(context.Context) error {
	f.stops.Add(1)
	if f.done != nil {
		close(f.done)
	}
	return nil
}

func TestMachineStopAuthorityAndIdempotence(t *testing.T) {
	fake := &fakeMachinePower{done: make(chan struct{})}
	var disconnected atomic.Bool
	p := &machinePower{provider: fake, disconnect: func() { disconnected.Store(true) }}
	for _, caller := range []bus.CallerIdentity{{}, {Scope: "view"}, {Scope: "triage"}, {PluginID: "plugin"}, {Trusted: true, Scope: "provider"}} {
		if _, err := machineStop(p)(caller, nil); err == nil {
			t.Fatalf("accepted unauthorized caller: %+v", caller)
		}
	}
	operator := bus.CallerIdentity{Trusted: true, Scope: "operator"}
	for i := 0; i < 3; i++ {
		if _, err := machineStop(p)(operator, json.RawMessage(`{"app":"other","machineId":"other","signal":"SIGKILL"}`)); err != nil {
			t.Fatal(err)
		}
	}
	select {
	case <-fake.done:
	case <-time.After(3 * time.Second):
		t.Fatal("stop never ran")
	}
	if fake.stops.Load() != 1 || !disconnected.Load() {
		t.Fatal("stop did not disconnect first, or duplicated")
	}
}

func TestMachineStopRefusesUnavailableProviderBeforeDisconnect(t *testing.T) {
	p := &machinePower{provider: &fakeMachinePower{checkErr: errors.New("secret-token")}, disconnect: func() { t.Error("disconnected on failed preflight") }}
	_, err := machineStop(p)(bus.CallerIdentity{Trusted: true, Scope: "operator"}, nil)
	if err == nil || strings.Contains(err.Error(), "secret-token") {
		t.Fatalf("bad error: %v", err)
	}
	if p.stopping {
		t.Fatal("failed preflight armed stop")
	}
}

func TestFlySelfStopUsesOnlyConfiguredCoordinatesAndGrace(t *testing.T) {
	var path string
	var body map[string]string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		path = r.URL.Path
		if r.Header.Get("Authorization") != "Bearer test-secret" {
			t.Error("missing credential")
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer server.Close()
	client := flyapi.New("test-secret")
	client.BaseURL = server.URL
	p := flyMachinePower{client: client, app: "own-app", id: "own-id"}
	if err := p.Stop(context.Background()); err != nil {
		t.Fatal(err)
	}
	if path != "/v1/apps/own-app/machines/own-id/stop" || body["signal"] != "SIGTERM" || body["timeout"] != "45s" {
		t.Fatalf("%s %+v", path, body)
	}
}

func TestIdleStopProtectsEveryScheduledJobAndUnknownFleet(t *testing.T) {
	now := time.Now()
	for _, job := range []quiescence.Job{
		{ActionKind: "shell", Running: true},
		{ActionKind: "shell", NextRun: now.Add(24 * time.Hour)},
		{ActionKind: "spawn", NextRun: now.Add(24 * time.Hour)},
	} {
		in := idlePowerInputs(quiescence.Inputs{Now: now, Jobs: []quiescence.Job{job}})
		if len(quiescence.Evaluate(in, quiescence.Tunables{})) == 0 {
			t.Fatalf("job was not protected: %+v", job)
		}
	}
	in := idlePowerInputs(quiescence.Inputs{Now: now, SessionsErr: errors.New("offline")})
	if len(quiescence.Evaluate(in, quiescence.Tunables{})) == 0 {
		t.Fatal("unknown fleet was treated as idle")
	}
}

func TestIdleStopRechecksBeforeDisconnect(t *testing.T) {
	fake := &fakeMachinePower{}
	p := &machinePower{provider: fake, stopping: true, disconnect: func() { t.Fatal("new work was interrupted") }}
	p.stop(func() bool { return false })
	if p.stopping || fake.stops.Load() != 0 {
		t.Fatal("idle stop was not cancelled")
	}
}

func TestMachinePowerRequiresExplicitWakeAndCredentials(t *testing.T) {
	t.Setenv("WKS_MACHINE_POWER", "fly")
	t.Setenv("WKS_MACHINE_WAKE", "")
	t.Setenv("FLY_APP_NAME", "app")
	t.Setenv("FLY_MACHINE_ID", "machine")
	t.Setenv("FLY_API_TOKEN", "secret")
	if configuredMachinePower(func() {}).provider != nil {
		t.Fatal("power enabled with no wake route")
	}
	t.Setenv("WKS_MACHINE_WAKE", "http")
	if configuredMachinePower(func() {}).provider == nil {
		t.Fatal("configured Fly provider absent")
	}
	t.Setenv("FLY_API_TOKEN", "")
	if configuredMachinePower(func() {}).provider != nil {
		t.Fatal("power enabled without credential")
	}
}

func TestAutomaticStopWaitsForContinuousQuietAndResetsOnWork(t *testing.T) {
	fake := &fakeMachinePower{done: make(chan struct{})}
	p := &machinePower{provider: fake, disconnect: func() {}}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	var working atomic.Bool
	working.Store(true)
	read := func(context.Context) quiescence.Inputs {
		mode := "input"
		if working.Load() {
			mode = "responding"
		}
		return quiescence.Inputs{Now: time.Now(), Sessions: []quiescence.Session{{ID: "agent", Mode: mode}}}
	}
	go p.runIdle(ctx, read, 100*time.Millisecond, 5*time.Millisecond)
	// Busy longer than the idle dwell must never stop.
	time.Sleep(150 * time.Millisecond)
	if fake.stops.Load() != 0 {
		t.Fatal("stopped while agent was working")
	}
	working.Store(false)
	time.Sleep(40 * time.Millisecond)
	working.Store(true)
	time.Sleep(30 * time.Millisecond)
	p.mu.Lock()
	calm := p.idleState.CalmSeconds
	p.mu.Unlock()
	if calm != 0 {
		t.Fatal("work did not reset quiet period")
	}
	working.Store(false)
	select {
	case <-fake.done:
	case <-time.After(3 * time.Second):
		t.Fatal("quiet machine never stopped")
	}
	if fake.stops.Load() != 1 {
		t.Fatal("duplicate automatic stop")
	}
}

func TestObservationModeMeasuresIdleWithoutCallingPowerProvider(t *testing.T) {
	fake := &fakeMachinePower{}
	p := &machinePower{provider: fake, observeOnly: true, disconnect: func() { t.Error("observation disconnected clients") }}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go p.runIdle(ctx, func(context.Context) quiescence.Inputs { return quiescence.Inputs{Now: time.Now()} }, 20*time.Millisecond, 5*time.Millisecond)
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		p.mu.Lock()
		quiet := p.idleState.Quiescent
		p.mu.Unlock()
		if quiet {
			if fake.stops.Load() != 0 {
				t.Fatal("observation stopped the machine")
			}
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("observation never reported quiet")
}
