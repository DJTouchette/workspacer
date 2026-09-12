package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestSpawnSetupCanBrowseConfiguredInactiveProjectWithoutContentAccess(t *testing.T) {
	tempConfigHome(t)
	reg := registryWithCwds(t)
	project := t.TempDir()
	if err := os.Mkdir(filepath.Join(project, "child"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(project, "private.txt"), []byte("unchanged"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := reg.cfg.save(map[string]any{"projects": map[string]any{project: map[string]any{"name": "Inactive project"}}}); err != nil {
		t.Fatal(err)
	}
	params, _ := json.Marshal(map[string]string{"path": project})
	raw, err := reg.handle(context.Background(), "fs.listDir", params)
	if err != nil {
		t.Fatal(err)
	}
	var listing listDirResult
	if err := json.Unmarshal(raw, &listing); err != nil {
		t.Fatal(err)
	}
	if len(listing.Dirs) != 1 || listing.Dirs[0] != "child" {
		t.Fatalf("wrong listing: %+v", listing)
	}
	params, _ = json.Marshal(map[string]string{"path": filepath.Join(project, "private.txt"), "contents": "changed"})
	for _, method := range []string{"fs.read", "fs.write"} {
		if _, err := reg.handle(context.Background(), method, params); err == nil {
			t.Fatalf("configured project granted %s", method)
		}
	}
	got, _ := os.ReadFile(filepath.Join(project, "private.txt"))
	if string(got) != "unchanged" {
		t.Fatal("content changed")
	}
}
