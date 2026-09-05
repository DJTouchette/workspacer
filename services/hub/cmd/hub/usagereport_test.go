package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

func TestUsageReportReadAndConcurrentCache(t *testing.T) {
	var calls atomic.Int32
	body := liveReportBody(t)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/usage/report" {
			t.Errorf("unexpected request: %s", r.URL.Path)
		}
		calls.Add(1)
		w.Write(body)
	}))
	defer server.Close()
	handler := usageReport(routing.New("", nil), newUsageWatcher(server.URL), nil)
	var wg sync.WaitGroup
	for range 12 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			out, err := handler(bus.CallerIdentity{}, json.RawMessage(`{}`))
			if err != nil {
				t.Error(err)
				return
			}
			if out.(limits.UsageProjection).ValidUntil == 0 {
				t.Error("no validity")
			}
		}()
	}
	wg.Wait()
	if calls.Load() != 1 {
		t.Fatalf("concurrent/cached readers made %d fetches", calls.Load())
	}
	if _, err := handler(bus.CallerIdentity{}, json.RawMessage(`{"url":"http://other"}`)); err == nil {
		t.Fatal("accepted caller parameters")
	}
	allowed := false
	for _, m := range authtoken.ScopeView.Methods() {
		if m == "usage.report" {
			allowed = true
		}
	}
	if !allowed {
		t.Fatal("view cannot read Overview")
	}
}
func TestUsageReportTransportFailure(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(503) }))
	defer server.Close()
	out, err := usageReport(routing.New("", nil), newUsageWatcher(server.URL), nil)(bus.CallerIdentity{}, nil)
	if err == nil || out != nil {
		t.Fatal("failed transport became a successful observation")
	}
}
func TestUsageReportHasNoDecisionSideEffects(t *testing.T) {
	raw, err := os.ReadFile("usagereport.go")
	if err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"routing.Select(", "routingSelect(", "RefreshAvailability(", ".Publish(", "exec.", "DecisionLog"} {
		if strings.Contains(string(raw), bad) {
			t.Fatalf("usage read gained decision side effect: %s", bad)
		}
	}
}
