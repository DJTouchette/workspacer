package main

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

// stubUIResolver maps one plugin id to a ui dir.
type stubUIResolver struct {
	id  string
	dir string
}

func (s stubUIResolver) UIDir(id string) (string, bool) {
	if id == s.id {
		return s.dir, true
	}
	return "", false
}

func TestPluginUIHandler(t *testing.T) {
	plugDir := t.TempDir()
	uiDir := filepath.Join(plugDir, "dist")
	if err := os.MkdirAll(uiDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(uiDir, "index.html"), []byte("<h1>hi</h1>"), 0o644); err != nil {
		t.Fatal(err)
	}
	// A secret sitting in the plugin root, outside the served ui dir.
	if err := os.WriteFile(filepath.Join(plugDir, ".bus-token"), []byte("SECRET"), 0o600); err != nil {
		t.Fatal(err)
	}

	h := pluginUIHandler(stubUIResolver{id: "acme.editor", dir: uiDir})

	get := func(path string) (*httptest.ResponseRecorder, string) {
		req := httptest.NewRequest(http.MethodGet, path, nil)
		rec := httptest.NewRecorder()
		h(rec, req)
		return rec, rec.Body.String()
	}

	// The directory form (what the renderer requests) serves index.html directly,
	// with no redirect — so the busToken query the webview carries isn't dropped.
	if rec, body := get("/plugins/ui/acme.editor/"); rec.Code != 200 || body != "<h1>hi</h1>" {
		t.Fatalf("dir index: code=%d body=%q", rec.Code, body)
	}

	// Unknown plugin → 404.
	if rec, _ := get("/plugins/ui/nope/index.html"); rec.Code != http.StatusNotFound {
		t.Fatalf("unknown plugin: code=%d, want 404", rec.Code)
	}

	// Traversal toward the secret in the plugin root must not be served.
	for _, p := range []string{
		"/plugins/ui/acme.editor/../.bus-token",
		"/plugins/ui/acme.editor/..%2f.bus-token",
	} {
		rec, body := get(p)
		if rec.Code == 200 && body == "SECRET" {
			t.Fatalf("traversal %q leaked the secret", p)
		}
	}
}
