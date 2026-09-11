package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/taskartifacts"
)

func TestTaskHandoffOwnerScopedBusAndRequiredBytes(t *testing.T) {
	reg := &registry{handoffRoot: t.TempDir()}
	a := taskartifacts.RepositoryBinding{ID: "fixture-binding-aa", Revision: "1", Repository: t.TempDir(), Remote: "https://repository.example.test/approved.git", RefPrefix: "refs/heads/wks-transfer", Owner: bus.TokenFingerprint("operator-a"), Origin: "fixture-origin-aa", Import: true, Export: true}
	b := a
	b.ID, b.Owner, b.Origin = "fixture-binding-bb", bus.TokenFingerprint("operator-b"), "fixture-origin-bb"
	fixtureWriteBindings(t, reg.handoffRoot, a, b)
	server := bus.NewServer(broker.New())
	server.SetToken("fixture-host")
	server.SetScopedTokenLookup(func(token string) (bus.ScopedIdent, bool) {
		if token == "operator-a" || token == "operator-b" {
			return bus.ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		}
		if token == "observer" {
			return bus.ScopedIdent{Scope: "view", Methods: authtoken.ScopeView.Methods()}, true
		}
		return bus.ScopedIdent{}, false
	})
	server.RegisterLocal("agents.taskHandoff", func(raw json.RawMessage) (any, error) {
		return reg.handle(context.Background(), "agents.taskHandoff", raw)
	})
	host := httptest.NewServer(server.Handler())
	defer host.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	client := func(token string) *busclient.Client {
		c := busclient.New(strings.Replace(host.URL, "http://", "ws://", 1)+"/bus?peer=1", token)
		go c.Run(ctx)
		return c
	}
	ca, cb, view := client("operator-a"), client("operator-b"), client("observer")
	task := strings.Repeat("a", 32)
	manifest := taskartifacts.Manifest{Version: 1, Task: task, Origin: a.Origin, Producer: task, Commit: strings.Repeat("b", 40), ObjectFormat: "sha1", Entries: []taskartifacts.Entry{{Name: "brief.md", Kind: "report", Size: 5, SHA256: taskartifacts.Digest([]byte("hello"))}}}
	plan := handoffPlan{Version: 1, Binding: a.ID, Revision: "1", Provider: "claude", Input: manifest}
	reserve := handoffRequest{Operation: "reserve", Binding: a.ID, Task: task, Plan: &plan, OriginKey: b.Owner}
	if _, err := ca.Call(ctx, "agents.taskHandoff", reserve); err != nil {
		t.Fatal(err)
	}
	if _, err := cb.Call(ctx, "agents.taskHandoff", reserve); err == nil {
		t.Fatal("other owner borrowed binding")
	}
	if _, err := view.Call(ctx, "agents.taskHandoff", reserve); err == nil {
		t.Fatal("observer admitted task transfer")
	}
	prepare := handoffRequest{Operation: "prepare", Binding: a.ID, Task: task}
	if _, err := ca.Call(ctx, "agents.taskHandoff", prepare); err == nil {
		t.Fatal("missing required bytes prepared")
	}
	write := handoffRequest{Operation: "write", Binding: a.ID, Task: task, Direction: "input", Data: []byte("wrong")}
	if _, err := ca.Call(ctx, "agents.taskHandoff", write); err != nil {
		t.Fatal(err)
	}
	if _, err := ca.Call(ctx, "agents.taskHandoff", prepare); err == nil {
		t.Fatal("corrupt required bytes prepared")
	}
	write.Data = []byte("hello")
	if _, err := ca.Call(ctx, "agents.taskHandoff", write); err == nil {
		t.Fatal("conflicting retransmission accepted")
	}
	plan.Binding, plan.Input.Origin = b.ID, b.Origin
	reserve.Binding, reserve.Plan = b.ID, &plan
	if _, err := cb.Call(ctx, "agents.taskHandoff", reserve); err != nil {
		t.Fatal("second origin collision", err)
	}
	if reg.handoffDir(a, task) == reg.handoffDir(b, task) {
		t.Fatal("origins share storage")
	}
	ra, _ := a.Ref(task, "input")
	rb, _ := b.Ref(task, "input")
	if ra == rb {
		t.Fatal("origins share Git ref")
	}
	for _, raw := range []string{
		`{"operation":"status","Operation":"freeze"}`,
		`{"operation":"status","operation":"freeze"}`,
		`{"Operation":"status"}`,
	} {
		if _, err := ca.Call(ctx, "agents.taskHandoff", json.RawMessage(raw)); err == nil {
			t.Fatal("aliased wire admitted")
		}
	}
}
