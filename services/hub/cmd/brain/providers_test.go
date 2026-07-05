package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// ── providers.checkAll probes PATH + honors config overrides ────────────────

func TestCheckAllProvidersDetectsOnPath(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("PATH binary-name probing is unix-shaped in this test")
	}
	dir := t.TempDir()
	// A fake `codex` on PATH; the others stay missing.
	bin := filepath.Join(dir, "codex")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir)

	got := checkAllProviders(map[string]string{})
	byName := map[string]providerStatus{}
	for _, s := range got {
		byName[s.Provider] = s
	}
	if len(byName) != 4 {
		t.Fatalf("expected all four providers reported, got %d: %+v", len(byName), got)
	}
	if c := byName["codex"]; !c.Found || c.ResolvedPath == nil || *c.ResolvedPath != bin {
		t.Errorf("codex should be found at %s, got %+v", bin, c)
	}
	if o := byName["opencode"]; o.Found || o.ResolvedPath != nil {
		t.Errorf("opencode should be missing (resolvedPath null), got %+v", o)
	}
}

func TestCheckAllProvidersHonorsCustomBin(t *testing.T) {
	dir := t.TempDir()
	custom := filepath.Join(dir, "my-pi")
	if err := os.WriteFile(custom, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", "") // nothing on PATH; only the override resolves

	got := checkAllProviders(map[string]string{"pi": custom})
	var pi providerStatus
	for _, s := range got {
		if s.Provider == "pi" {
			pi = s
		}
	}
	if !pi.Found || pi.ResolvedPath == nil || *pi.ResolvedPath != custom || pi.CustomBin != custom {
		t.Errorf("pi override should resolve to %s, got %+v", custom, pi)
	}
}

// resolvedPath must serialize as JSON null (not "") when a provider is missing,
// since the renderer types it `resolvedPath: string | null`.
func TestProviderStatusMissingSerializesNull(t *testing.T) {
	t.Setenv("PATH", "")
	raw, err := jsonResult(checkAllProviders(map[string]string{}))
	if err != nil {
		t.Fatal(err)
	}
	var rows []map[string]any
	if err := json.Unmarshal(raw, &rows); err != nil {
		t.Fatal(err)
	}
	for _, row := range rows {
		if v, ok := row["resolvedPath"]; !ok || v != nil {
			t.Errorf("missing provider %v should have resolvedPath: null, got %v", row["provider"], v)
		}
	}
}

// ── providers.listModels relays to claudemon and unwraps { models } ─────────

func TestProvidersListModelsRelaysAndUnwraps(t *testing.T) {
	var gotPath string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		_ = json.NewEncoder(w).Encode(map[string]any{
			"models": []map[string]any{
				{"id": "gpt-x", "label": "GPT X", "default": true},
				{"id": "gpt-y", "label": "GPT Y"},
			},
		})
	}))
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	res, err := reg.handle(context.Background(), "providers.listModels", []byte(`{"provider":"codex","cwd":"/tmp"}`))
	if err != nil {
		t.Fatal(err)
	}
	if gotPath != "/providers/codex/models" {
		t.Errorf("expected relay to /providers/codex/models, hit %q", gotPath)
	}
	var models []map[string]any
	if err := json.Unmarshal(res, &models); err != nil {
		t.Fatalf("expected a bare array back, got %s (err %v)", res, err)
	}
	if len(models) != 2 || models[0]["id"] != "gpt-x" || models[0]["default"] != true || models[1]["default"] != false {
		t.Fatalf("unexpected models payload: %+v", models)
	}
}

func TestProvidersListModelsRejectsBadProvider(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	if _, err := reg.handle(context.Background(), "providers.listModels", []byte(`{"provider":"claude"}`)); err == nil {
		t.Fatal("claude is not a managed provider; listModels should reject it")
	}
}

// A provider CLI that errors (claudemon 5xx) soft-fails to an empty array so the
// Spawn dialog falls back to free-text, never a hard error.
func TestProvidersListModelsSoftFailsToEmpty(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusBadGateway)
	}))
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	res, err := reg.handle(context.Background(), "providers.listModels", []byte(`{"provider":"opencode"}`))
	if err != nil {
		t.Fatalf("a failing provider CLI must not error the call: %v", err)
	}
	var models []map[string]any
	if err := json.Unmarshal(res, &models); err != nil || len(models) != 0 {
		t.Fatalf("expected an empty array on provider failure, got %s (err %v)", res, err)
	}
}
