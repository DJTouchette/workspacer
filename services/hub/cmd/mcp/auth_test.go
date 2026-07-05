package main

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/busclient"
)

func TestCheckBindPolicy(t *testing.T) {
	cases := []struct {
		name    string
		addr    string
		token   string
		wantErr bool
	}{
		{"loopback ip, no token, allowed", "127.0.0.1:7897", "", false},
		{"loopback name, no token, allowed", "localhost:7897", "", false},
		{"ipv6 loopback, no token, allowed", "[::1]:7897", "", false},
		{"bare port, no token, refused", ":7897", "", true},
		{"all interfaces, no token, refused", "0.0.0.0:7897", "", true},
		{"lan ip, no token, refused", "192.168.1.10:7897", "", true},
		{"lan ip, with token, allowed", "192.168.1.10:7897", "s3cret", false},
		{"all interfaces, with token, allowed", "0.0.0.0:7897", "s3cret", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			err := checkBindPolicy(c.addr, c.token)
			if (err != nil) != c.wantErr {
				t.Fatalf("checkBindPolicy(%q, tokenSet=%v) err = %v, wantErr = %v", c.addr, c.token != "", err, c.wantErr)
			}
		})
	}
}

func TestIsLoopbackAddr(t *testing.T) {
	cases := []struct {
		addr string
		want bool
	}{
		{"127.0.0.1:7897", true},
		{"localhost:7897", true},
		{"[::1]:7897", true},
		{"127.0.0.1", true}, // no port
		{":7897", false},
		{"0.0.0.0:7897", false},
		{"::", false},
		{"192.168.1.10:7897", false},
		{"example.com:7897", false}, // unresolved hostname → fail safe
	}
	for _, c := range cases {
		if got := isLoopbackAddr(c.addr); got != c.want {
			t.Errorf("isLoopbackAddr(%q) = %v, want %v", c.addr, got, c.want)
		}
	}
}

func TestRequireBearer(t *testing.T) {
	ok := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
	})

	// No token configured → passthrough, no auth required.
	t.Run("no token passes through", func(t *testing.T) {
		rec := httptest.NewRecorder()
		requireBearer("", ok).ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/mcp", nil))
		if rec.Code != http.StatusOK {
			t.Fatalf("code = %d, want 200", rec.Code)
		}
	})

	h := requireBearer("s3cret", ok)

	t.Run("missing header is 401", func(t *testing.T) {
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/mcp", nil))
		if rec.Code != http.StatusUnauthorized {
			t.Fatalf("code = %d, want 401", rec.Code)
		}
	})

	t.Run("wrong token is 401", func(t *testing.T) {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, "/mcp", nil)
		req.Header.Set("Authorization", "Bearer nope")
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusUnauthorized {
			t.Fatalf("code = %d, want 401", rec.Code)
		}
	})

	t.Run("correct token is 200", func(t *testing.T) {
		rec := httptest.NewRecorder()
		req := httptest.NewRequest(http.MethodGet, "/mcp", nil)
		req.Header.Set("Authorization", "Bearer s3cret")
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("code = %d, want 200", rec.Code)
		}
	})
}

// TestMuxHealthOpenMCPGuarded proves the wiring: /health is reachable without a
// token even when auth is on, while /mcp demands the bearer.
func TestMuxHealthOpenMCPGuarded(t *testing.T) {
	client := busclient.New("ws://127.0.0.1:0/bus", "")
	mux := newMux(newServer(client), client, "s3cret")
	srv := httptest.NewServer(mux)
	defer srv.Close()

	// /health is open.
	resp, err := http.Get(srv.URL + "/health")
	if err != nil {
		t.Fatalf("health GET: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("/health status = %d, want 200 (must stay open)", resp.StatusCode)
	}

	// /mcp without a token is rejected before reaching the MCP handler.
	resp, err = http.Get(srv.URL + "/mcp")
	if err != nil {
		t.Fatalf("mcp GET: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("/mcp without token status = %d, want 401", resp.StatusCode)
	}

	// /mcp WITH the token passes auth (reaches the MCP handler, which no longer
	// answers 401 — a bare GET is a bad MCP request, so just assert not-401).
	req, _ := http.NewRequest(http.MethodGet, srv.URL+"/mcp", nil)
	req.Header.Set("Authorization", "Bearer s3cret")
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("mcp GET with token: %v", err)
	}
	resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized {
		t.Fatalf("/mcp with correct token was 401; auth should have passed")
	}
}
