package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// fakeClaudemon serves the two endpoints the status probe reads.
func fakeClaudemon(t *testing.T, sessions int) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(map[string]string{"status": "ok"})
	})
	mux.HandleFunc("/sessions", func(w http.ResponseWriter, _ *http.Request) {
		list := make([]map[string]string, sessions)
		for i := range list {
			list[i] = map[string]string{"session_id": "s"}
		}
		_ = json.NewEncoder(w).Encode(list)
	})
	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv
}

// fakeHub mimics the hub's /health: counts only for an authorized caller.
func fakeHub(t *testing.T, token string, methods int) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	mux.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if token != "" && r.Header.Get("Authorization") != "Bearer "+token {
			_ = json.NewEncoder(w).Encode(map[string]any{"status": "ok"})
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"status": "ok", "methods": methods})
	})
	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv
}

func TestProbeClaudemon(t *testing.T) {
	ctx := context.Background()

	t.Run("healthy with session count", func(t *testing.T) {
		srv := fakeClaudemon(t, 2)
		got := probeClaudemon(ctx, srv.URL)
		if !got.OK || !strings.Contains(got.Detail, "2 session(s)") {
			t.Errorf("probeClaudemon = %+v", got)
		}
	})

	t.Run("nothing listening is graceful", func(t *testing.T) {
		srv := fakeClaudemon(t, 0)
		srv.Close() // freed port — connection refused
		got := probeClaudemon(ctx, srv.URL)
		if got.OK {
			t.Errorf("dead claudemon reported OK: %+v", got)
		}
		if !strings.Contains(got.Detail, "not running") {
			t.Errorf("detail should say not running, got %q", got.Detail)
		}
	})
}

func TestProbeHub(t *testing.T) {
	ctx := context.Background()

	tests := []struct {
		name       string
		serverTok  string
		callerTok  string
		wantOK     bool
		wantDetail string
	}{
		{"authorized sees the method count", "sec", "sec", true, "capability method(s)"},
		{"wrong token still counts as alive", "sec", "nope", true, "token not accepted"},
		{"no auth configured exposes counts", "", "", true, "capability method(s)"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			srv := fakeHub(t, tt.serverTok, 17)
			got := probeHub(ctx, srv.URL, tt.callerTok)
			if got.OK != tt.wantOK || !strings.Contains(got.Detail, tt.wantDetail) {
				t.Errorf("probeHub = %+v, want OK=%v detail~%q", got, tt.wantOK, tt.wantDetail)
			}
		})
	}

	t.Run("nothing listening is graceful", func(t *testing.T) {
		srv := fakeHub(t, "", 0)
		srv.Close()
		got := probeHub(ctx, srv.URL, "")
		if got.OK || !strings.Contains(got.Detail, "not running") {
			t.Errorf("probeHub on a dead hub = %+v", got)
		}
	})
}

func TestRenderStatus(t *testing.T) {
	r := statusReport{
		Claudemon: componentStatus{OK: true, Detail: "healthy, 1 session(s)"},
		Hub:       componentStatus{OK: false, Detail: "not running (connection refused)"},
		Brain:     componentStatus{OK: false, Detail: "not checked (hub is down)"},
	}
	out := renderStatus(r, "http://127.0.0.1:7891", "http://127.0.0.1:7895")
	for _, want := range []string{"claudemon", "hub", "brain", "up", "down", "connection refused", "http://127.0.0.1:7891"} {
		if !strings.Contains(out, want) {
			t.Errorf("status output missing %q:\n%s", want, out)
		}
	}
}

func TestStatusReportJSONShape(t *testing.T) {
	raw, err := json.Marshal(statusReport{})
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"claudemon", "hub", "brain"} {
		if _, ok := m[key]; !ok {
			t.Errorf("status JSON missing key %q: %s", key, raw)
		}
	}
}
