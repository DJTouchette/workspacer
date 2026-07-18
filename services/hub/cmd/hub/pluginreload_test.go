package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

// stubAdder records the manifests handed to Add, standing in for *plugin.Manager
// so the reload handler is tested without a live sidecar/token registry.
type stubAdder struct{ added []plugin.Manifest }

func (s *stubAdder) Add(m plugin.Manifest) { s.added = append(s.added, m) }

func postReloadTo(h http.HandlerFunc, jsonBody string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(http.MethodPost, "/plugins/reload", strings.NewReader(jsonBody))
	rec := httptest.NewRecorder()
	h(rec, req)
	return rec
}

func TestPluginReloadHandlerLoadsAndAdds(t *testing.T) {
	plugDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(plugDir, "plugin.json"),
		[]byte(`{"id":"acme.dev","name":"Acme","apiVersion":"1"}`), 0o644); err != nil {
		t.Fatal(err)
	}

	add := &stubAdder{}
	h := pluginReloadHandler(add)

	rec := postReloadTo(h, `{"dir":`+jsonString(plugDir)+`}`)
	if rec.Code != http.StatusOK {
		t.Fatalf("code=%d body=%q, want 200", rec.Code, rec.Body.String())
	}
	var out struct {
		OK bool   `json:"ok"`
		ID string `json:"id"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatal(err)
	}
	if !out.OK || out.ID != "acme.dev" {
		t.Fatalf("response = %+v, want ok + id acme.dev", out)
	}
	if len(add.added) != 1 || add.added[0].ID != "acme.dev" {
		t.Fatalf("Add called with %+v, want exactly acme.dev", add.added)
	}
	// The handler must pass the real dir through so mgr.Add resolves paths there.
	if add.added[0].Dir != plugDir {
		t.Fatalf("manifest Dir = %q, want %q", add.added[0].Dir, plugDir)
	}
}

func TestPluginReloadHandlerRejectsBadInput(t *testing.T) {
	add := &stubAdder{}
	h := pluginReloadHandler(add)

	// Missing dir → 400.
	if rec := postReloadTo(h, `{}`); rec.Code != http.StatusBadRequest {
		t.Fatalf("missing dir: code=%d, want 400", rec.Code)
	}
	// A dir with no plugin.json → 400 (plugin.Load fails), and Add is never called.
	empty := t.TempDir()
	if rec := postReloadTo(h, `{"dir":`+jsonString(empty)+`}`); rec.Code != http.StatusBadRequest {
		t.Fatalf("bad dir: code=%d, want 400", rec.Code)
	}
	if len(add.added) != 0 {
		t.Fatalf("Add was called on invalid input: %+v", add.added)
	}
}

// jsonString quotes s as a JSON string literal (handles Windows backslashes in
// temp paths, which would otherwise be invalid JSON escapes).
func jsonString(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}
