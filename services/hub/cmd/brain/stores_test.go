package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestSlugs(t *testing.T) {
	cases := []struct {
		fn   func(string) string
		in   string
		want string
	}{
		{slugLayout, "My Layout!", "my-layout"},
		{slugLayout, "  spaced  ", "spaced"},
		{slugLayout, "a//b**c", "a-b-c"},
		{slugLayout, "!!!", "layout"}, // empty after trim → fallback
		{slugSession, "My Session", "my-session"},
		{slugSession, "keep_under-score", "keep_under-score"},
	}
	for _, c := range cases {
		if got := c.fn(c.in); got != c.want {
			t.Errorf("slug(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestLayoutsSaveListDelete(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	saved, err := saveLayout(map[string]any{
		"name":   "My Layout",
		"agents": []any{map[string]any{"name": "a", "cwd": "/tmp", "tabs": []any{}}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if saved["id"] != "my-layout" || saved["createdAt"] == "" {
		t.Fatalf("unexpected saved layout: %+v", saved)
	}
	if _, err := os.Stat(filepath.Join(dir, "workspacer", "layouts", "my-layout.yaml")); err != nil {
		t.Fatalf("layout file not written: %v", err)
	}

	// A malformed file (no agents) is ignored by list.
	_ = os.WriteFile(filepath.Join(dir, "workspacer", "layouts", "junk.yaml"), []byte("name: junk\n"), 0o644)

	list := listLayouts()
	if len(list) != 1 || list[0]["id"] != "my-layout" {
		t.Fatalf("expected only the valid layout, got %+v", list)
	}

	// remove re-slugs the id and unlinks the matching file.
	removeLayout("My Layout")
	if len(listLayouts()) != 0 {
		t.Fatal("layout should be gone after remove")
	}
}

func TestLayoutListSortsNewestFirst(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	ldir := filepath.Join(dir, "workspacer", "layouts")
	_ = os.MkdirAll(ldir, 0o755)
	_ = os.WriteFile(filepath.Join(ldir, "old.yaml"), []byte("id: old\ncreatedAt: \"2020-01-01T00:00:00.000Z\"\nagents: []\n"), 0o644)
	_ = os.WriteFile(filepath.Join(ldir, "new.yaml"), []byte("id: new\ncreatedAt: \"2024-01-01T00:00:00.000Z\"\nagents: []\n"), 0o644)

	list := listLayouts()
	if len(list) != 2 || list[0]["id"] != "new" {
		t.Fatalf("expected newest first, got %+v", list)
	}
}

func TestSavedSessionsRoundTrip(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	reg := newRegistry(newClaudemonClient("http://unused"))
	ctx := context.Background()

	// Save an agent-centric session with one global agent (excluded from count).
	params := `{"name":"Work","activeAgentId":"a1","agents":[
		{"id":"a1","tabs":[{"panes":[{},{}]}]},
		{"id":"g","global":true,"tabs":[{"panes":[{}]}]}
	]}`
	res, err := reg.handle(ctx, "sessions.save", json.RawMessage(params))
	if err != nil {
		t.Fatal(err)
	}
	var filename string
	_ = json.Unmarshal(res, &filename)
	if filename != "work.yaml" {
		t.Fatalf("filename = %q, want work.yaml", filename)
	}

	// list reports counts: 3 panes total, 1 non-global agent.
	listRes, _ := reg.handle(ctx, "sessions.list", nil)
	var list []sessionListEntry
	_ = json.Unmarshal(listRes, &list)
	if len(list) != 1 || list[0].PaneCount != 3 || list[0].AgentCount != 1 {
		t.Fatalf("unexpected list entry: %+v", list)
	}
	if list[0].Timestamp == "" {
		t.Error("save should stamp a timestamp")
	}

	// load returns the blob; a missing file returns JSON null.
	loadRes, _ := reg.handle(ctx, "sessions.load", json.RawMessage(`{"filename":"work.yaml"}`))
	var loaded map[string]any
	if err := json.Unmarshal(loadRes, &loaded); err != nil || loaded["name"] != "Work" {
		t.Fatalf("load returned %s (err %v)", loadRes, err)
	}
	missing, _ := reg.handle(ctx, "sessions.load", json.RawMessage(`{"filename":"nope.yaml"}`))
	if string(missing) != "null" {
		t.Errorf("missing session should load as null, got %s", missing)
	}

	// delete removes it.
	if _, err := reg.handle(ctx, "sessions.delete", json.RawMessage(`{"filename":"work.yaml"}`)); err != nil {
		t.Fatal(err)
	}
	listRes2, _ := reg.handle(ctx, "sessions.list", nil)
	var list2 []sessionListEntry
	_ = json.Unmarshal(listRes2, &list2)
	if len(list2) != 0 {
		t.Fatalf("session should be deleted, got %+v", list2)
	}
}

func TestPaneCountLegacyAndFlat(t *testing.T) {
	legacy := map[string]any{"tabs": []any{
		map[string]any{"panes": []any{map[string]any{}}},
		map[string]any{"panes": []any{map[string]any{}, map[string]any{}}},
	}}
	if got := paneCount(legacy); got != 3 {
		t.Errorf("legacy paneCount = %d, want 3", got)
	}
	flat := map[string]any{"panes": []any{map[string]any{}, map[string]any{}}}
	if got := paneCount(flat); got != 2 {
		t.Errorf("flat paneCount = %d, want 2", got)
	}
}

// TestSavedSessionPathContainment proves loadSavedSession/deleteSavedSession
// reject a client-supplied traversal filename instead of reading or removing a
// file outside the sessions directory (filepath.Join runs Clean, which collapses
// ".." rather than blocking it). Covers idx 14.
func TestSavedSessionPathContainment(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		t.Fatal(err)
	}

	// A secret file OUTSIDE the sessions directory (sibling of the config dir).
	// sessionsDir() == <dir>/workspacer/sessions, so ../../ lands at <dir>.
	secret := filepath.Join(dir, "secret.yaml")
	if err := os.WriteFile(secret, []byte("name: secret\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	traversal := filepath.Join("..", "..", "secret.yaml")

	// load must NOT read a file outside the sessions dir.
	if got := loadSavedSession(traversal); got != nil {
		t.Fatalf("loadSavedSession leaked out-of-dir file: %+v", got)
	}

	// delete must NOT remove a file outside the sessions dir.
	deleteSavedSession(traversal)
	if _, err := os.Stat(secret); err != nil {
		t.Fatalf("deleteSavedSession removed out-of-dir file: %v", err)
	}
}
