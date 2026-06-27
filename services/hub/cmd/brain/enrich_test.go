package main

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
)

func TestEnrichSnapshotSpawnMeta(t *testing.T) {
	meta := newMetaStore()
	meta.set("s1", spawnMeta{Label: "My Agent", ParentSessionID: "p1", IsSupervisor: true})

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/x","mode":"input"}`), meta)
	var m map[string]any
	_ = json.Unmarshal(out, &m)
	if m["label"] != "My Agent" || m["parentSessionId"] != "p1" || m["isSupervisor"] != true {
		t.Fatalf("spawn metadata not overlaid: %v", m)
	}
	if m["mode"] != "input" {
		t.Error("original fields must be preserved")
	}
}

func TestEnrichSnapshotCwdName(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	writeFile(t, filepath.Join(dir, "workspacer", "tui-names.json"), `{"/proj":"Renamed"}`)

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/proj"}`), newMetaStore())
	var m map[string]any
	_ = json.Unmarshal(out, &m)
	if m["label"] != "Renamed" {
		t.Fatalf("cwd rename not applied, got %v", m["label"])
	}
}

func TestEnrichSpawnLabelWinsOverCwdName(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	writeFile(t, filepath.Join(dir, "workspacer", "tui-names.json"), `{"/proj":"FromFile"}`)
	meta := newMetaStore()
	meta.set("s1", spawnMeta{Label: "FromSpawn"})

	out := enrichSnapshot(json.RawMessage(`{"session_id":"s1","cwd":"/proj"}`), meta)
	var m map[string]any
	_ = json.Unmarshal(out, &m)
	if m["label"] != "FromSpawn" {
		t.Fatalf("spawn label should win, got %v", m["label"])
	}
}

func TestSpawnRecordsMeta(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.meta = newMetaStore()

	res, err := reg.handle(context.Background(), "agents.spawn",
		json.RawMessage(`{"cwd":"/tmp","label":"Worker","parentSessionId":"boss"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out struct {
		SessionID string `json:"sessionId"`
	}
	_ = json.Unmarshal(res, &out)
	m, ok := reg.meta.get(out.SessionID)
	if !ok || m.Label != "Worker" || m.ParentSessionID != "boss" {
		t.Fatalf("spawn should record metadata, got %+v (ok=%v)", m, ok)
	}
}

// The store applies enrichment as snapshots land.
func TestStoreEnrichesOnSet(t *testing.T) {
	meta := newMetaStore()
	meta.set("s1", spawnMeta{Label: "Named"})
	s := newSessionStore()
	s.enrich = func(snap json.RawMessage) json.RawMessage { return enrichSnapshot(snap, meta) }

	s.set("s1", json.RawMessage(`{"session_id":"s1"}`))
	snap, _ := s.get("s1")
	var m map[string]any
	_ = json.Unmarshal(snap, &m)
	if m["label"] != "Named" {
		t.Fatalf("store should enrich on set, got %v", m["label"])
	}
}
