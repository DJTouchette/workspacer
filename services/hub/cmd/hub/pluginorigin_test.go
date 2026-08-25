package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// A browser cannot be given a hub-served plugin's UI on the SAME origin as /app
// without also handing that plugin the app document: same origin means
// `parent.document`, `parent.window.electronAPI.*` and the host token in
// sessionStorage. The renderer therefore frames a same-origin plugin with an
// opaque sandbox, which costs the plugin its bus link.
//
// The only fix that does not loosen anything is a SECOND ORIGIN for the same
// hub — a distinct host or port the operator already routes here (a fly.io
// service on :8443, `tailscale serve --https=8443`, any reverse proxy). The hub
// cannot discover that by itself, so the operator declares it and the hub
// advertises it here. It is not a secret: it is a URL the operator chose to
// publish, and the client needs it before it can frame anything.
func TestPluginOriginHandlerAdvertisesTheDeclaredOrigin(t *testing.T) {
	rec := httptest.NewRecorder()
	pluginOriginHandler("https://plugins.example:8443")(rec, httptest.NewRequest(http.MethodGet, "/plugins/origin", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200", rec.Code)
	}
	var body struct {
		Origin string `json:"origin"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("decode: %v (body %q)", err, rec.Body.String())
	}
	if body.Origin != "https://plugins.example:8443" {
		t.Fatalf("origin = %q, want the declared one", body.Origin)
	}
}

// Unset is the norm, and must answer cleanly rather than 404 — the client
// distinguishes "no second origin" (fall back to same-origin framing) from "the
// hub did not answer" (retry), and a 404 body is not JSON.
func TestPluginOriginHandlerEmptyWhenUndeclared(t *testing.T) {
	rec := httptest.NewRecorder()
	pluginOriginHandler("")(rec, httptest.NewRequest(http.MethodGet, "/plugins/origin", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200", rec.Code)
	}
	if got := rec.Body.String(); got != "{\"origin\":\"\"}\n" && got != "{}\n" {
		t.Fatalf("body = %q, want an empty-origin JSON object", got)
	}
}

// The value ends up as an iframe `src` prefix. A non-absolute or non-http(s)
// value there is a script-injection shape (javascript:, data:), so it is
// rejected at startup rather than served to every client.
func TestNormalizePluginOriginRejectsNonHTTP(t *testing.T) {
	for _, bad := range []string{
		"javascript:alert(1)",
		"data:text/html,<script>",
		"plugins.example", // no scheme
		"/plugins",        // path, not an origin
		"ftp://plugins.example",
	} {
		if got, err := normalizePluginOrigin(bad); err == nil {
			t.Fatalf("normalizePluginOrigin(%q) = %q, want an error", bad, got)
		}
	}
}

// It is an ORIGIN: scheme://host[:port]. A trailing slash or a stray path is
// normalized away so the client can concatenate paths onto it blindly.
func TestNormalizePluginOriginKeepsOnlyTheOrigin(t *testing.T) {
	for in, want := range map[string]string{
		"https://plugins.example:8443/":     "https://plugins.example:8443",
		"https://plugins.example:8443":      "https://plugins.example:8443",
		"http://localhost:7896/plugins/ui/": "http://localhost:7896",
		"HTTPS://Plugins.Example":           "https://plugins.example",
	} {
		got, err := normalizePluginOrigin(in)
		if err != nil {
			t.Fatalf("normalizePluginOrigin(%q): %v", in, err)
		}
		if got != want {
			t.Fatalf("normalizePluginOrigin(%q) = %q, want %q", in, got, want)
		}
	}
}
