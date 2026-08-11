package main

// The --trusted-host flag is the ONLY thing that keeps the desktop's shipped
// "HTTPS via Tailscale" toggle from 403ing every route behind it, so the flag
// parsing and the SetTrustedHosts call are pinned together, end to end through
// the server the hub actually serves.

import (
	"net/http"
	"net/http/httptest"
	"reflect"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

func TestSplitTrustedHostsParsesTheFlag(t *testing.T) {
	got := splitTrustedHosts(" a.ts.net , ,b.example.com ")
	want := []string{"a.ts.net", "b.example.com"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("splitTrustedHosts = %v, want %v", got, want)
	}
	if got := splitTrustedHosts(""); len(got) != 0 {
		t.Fatalf("empty flag should yield no hosts, got %v", got)
	}
}

func TestConfigureTrustedHostsReachesTheServedHandler(t *testing.T) {
	srv := bus.NewServer(broker.New())
	http.DefaultServeMux = http.NewServeMux() // keep the global mux clean
	ts := httptest.NewServer(srv.Handler())
	defer ts.Close()

	probe := func() int {
		req, err := http.NewRequest(http.MethodGet, ts.URL+"/health", nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Host = "node.tail1234.ts.net" // what `tailscale serve` forwards
		res, err := ts.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer res.Body.Close()
		return res.StatusCode
	}

	if got := probe(); got != http.StatusForbidden {
		t.Fatalf("control: an undeclared proxy host must be 403, got %d", got)
	}
	configureTrustedHosts(srv, "node.tail1234.ts.net")
	if got := probe(); got == http.StatusForbidden {
		t.Fatal("--trusted-host was parsed but never installed on the server: every route behind the proxy still 403s")
	}
	// And clearing it puts the shape-only rule back.
	configureTrustedHosts(srv, "")
	if got := probe(); got != http.StatusForbidden {
		t.Fatalf("clearing --trusted-host must restore the pin, got %d", got)
	}
}
