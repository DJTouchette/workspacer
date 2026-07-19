package main

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPluginSDKHandler(t *testing.T) {
	req := httptest.NewRequest(http.MethodGet, "/plugins/sdk.js", nil)
	rec := httptest.NewRecorder()
	pluginSDKHandler()(rec, req)

	if rec.Code != 200 {
		t.Fatalf("code=%d, want 200", rec.Code)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "application/javascript; charset=utf-8" {
		t.Fatalf("content-type=%q", ct)
	}
	if cc := rec.Header().Get("Cache-Control"); cc == "" {
		t.Fatalf("expected a cache header")
	}
	if body := rec.Body.String(); !strings.Contains(body, "window.workspacer") {
		t.Fatalf("SDK body does not define window.workspacer")
	}
}

// The injected HTML seeds window.__WKS_SETTINGS__ from the merged settings, and a
// non-HTML asset is served byte-for-byte with no injection.
func TestPluginUIHandlerInjectionAndPassthrough(t *testing.T) {
	uiDir := t.TempDir()
	const indexHTML = "<html><head></head><body>page</body></html>"
	if err := os.WriteFile(filepath.Join(uiDir, "index.html"), []byte(indexHTML), 0o644); err != nil {
		t.Fatal(err)
	}
	const appJS = "console.log('hi');\nexport const x = 1;\n"
	if err := os.WriteFile(filepath.Join(uiDir, "app.js"), []byte(appJS), 0o644); err != nil {
		t.Fatal(err)
	}
	const styleCSS = "body{color:red}"
	if err := os.WriteFile(filepath.Join(uiDir, "style.css"), []byte(styleCSS), 0o644); err != nil {
		t.Fatal(err)
	}

	settingsFor := func(id string) map[string]any {
		if id == "acme.editor" {
			return map[string]any{"theme": "dark", "size": float64(12)}
		}
		return nil
	}
	h := pluginUIHandler(stubUIResolver{id: "acme.editor", dir: uiDir}, settingsFor)

	get := func(path string) (*httptest.ResponseRecorder, string) {
		req := httptest.NewRequest(http.MethodGet, path, nil)
		rec := httptest.NewRecorder()
		h(rec, req)
		return rec, rec.Body.String()
	}

	// index.html: injected before </head>, plugin id + merged settings seeded.
	rec, body := get("/plugins/ui/acme.editor/index.html")
	if rec.Code != 200 {
		t.Fatalf("index: code=%d", rec.Code)
	}
	if !strings.Contains(body, `<script src="/plugins/sdk.js">`) {
		t.Fatalf("index: sdk script not injected: %q", body)
	}
	if !strings.Contains(body, `window.__WKS_PLUGIN_ID__="acme.editor"`) {
		t.Fatalf("index: plugin id not seeded: %q", body)
	}
	if !strings.Contains(body, `window.__WKS_SETTINGS__=`) || !strings.Contains(body, `"theme":"dark"`) {
		t.Fatalf("index: merged settings not seeded: %q", body)
	}
	if hi, si, he := strings.Index(body, "__WKS_PLUGIN_ID__"), strings.Index(body, "/plugins/sdk.js"), strings.Index(body, "</head>"); hi < 0 || hi > he || si > he {
		t.Fatalf("index: injection must be before </head>: %q", body)
	}

	// Non-HTML assets pass through unmodified (byte-for-byte, no injection).
	if rec, body := get("/plugins/ui/acme.editor/app.js"); rec.Code != 200 || body != appJS {
		t.Fatalf("app.js: code=%d body=%q (want unmodified)", rec.Code, body)
	}
	if rec, body := get("/plugins/ui/acme.editor/style.css"); rec.Code != 200 || body != styleCSS {
		t.Fatalf("style.css: code=%d body=%q (want unmodified)", rec.Code, body)
	}
}

// An HTML-targeted request that tries to escape the ui dir must not read outside
// it — the injection read path preserves http.Dir's confinement.
func TestPluginUIHandlerHTMLTraversal(t *testing.T) {
	plugDir := t.TempDir()
	uiDir := filepath.Join(plugDir, "dist")
	if err := os.MkdirAll(uiDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(uiDir, "index.html"), []byte("<head></head>ok"), 0o644); err != nil {
		t.Fatal(err)
	}
	// A secret .html in the plugin root, one level above the served ui dir.
	if err := os.WriteFile(filepath.Join(plugDir, "secret.html"), []byte("TOPSECRET"), 0o644); err != nil {
		t.Fatal(err)
	}

	h := pluginUIHandler(stubUIResolver{id: "acme.editor", dir: uiDir}, nil)
	for _, p := range []string{
		"/plugins/ui/acme.editor/../secret.html",
		"/plugins/ui/acme.editor/..%2fsecret.html",
		"/plugins/ui/acme.editor/subdir/../../secret.html",
	} {
		req := httptest.NewRequest(http.MethodGet, p, nil)
		rec := httptest.NewRecorder()
		h(rec, req)
		if strings.Contains(rec.Body.String(), "TOPSECRET") {
			t.Fatalf("traversal %q leaked the secret html (code=%d)", p, rec.Code)
		}
	}
}
