package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestHeadlessFileWatchReplacementAndReferences(t *testing.T) {
	fx := newGitFixture(t)
	r := registryWithCwds(t, fx.agentCwd)
	var events []map[string]string
	r.publish = func(topic string, data json.RawMessage) {
		if topic != "fs.changed" {
			t.Fatalf("unexpected topic %s", topic)
		}
		var ev map[string]string
		if err := json.Unmarshal(data, &ev); err != nil {
			t.Fatal(err)
		}
		events = append(events, ev)
	}
	path := filepath.Join(fx.agentCwd, "tracked.ts")
	raw, _ := json.Marshal(map[string]string{"path": path})
	call := func(method string) {
		t.Helper()
		if _, err := r.handle(context.Background(), method, raw); err != nil {
			t.Fatal(err)
		}
	}
	call("fs.watch")
	call("fs.watch")
	replacement := filepath.Join(fx.agentCwd, "replace.tmp")
	if err := os.WriteFile(replacement, []byte("replacement\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Rename(replacement, path); err != nil {
		t.Fatal(err)
	}
	r.pollFileChanges(context.Background())
	if len(events) != 1 || events[0]["path"] != path || events[0]["eventType"] != "rename" {
		t.Fatalf("replacement event: %+v", events)
	}
	call("fs.unwatch")
	if err := os.WriteFile(path, []byte("changed again, after one reference left\n"), 0600); err != nil {
		t.Fatal(err)
	}
	r.pollFileChanges(context.Background())
	if len(events) != 2 || events[1]["eventType"] != "change" {
		t.Fatalf("remaining reference did not observe change: %+v", events)
	}
	call("fs.unwatch")
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	r.pollFileChanges(context.Background())
	if len(events) != 2 {
		t.Fatal("unwatched file emitted")
	}
}

func TestHeadlessFileWatchRefusesEscapeAndStopsOnSymlinkSwap(t *testing.T) {
	fx := newGitFixture(t)
	r := registryWithCwds(t, fx.agentCwd)
	count := 0
	r.publish = func(string, json.RawMessage) { count++ }
	outside := filepath.Join(fx.outside, "secret.txt")
	for _, method := range []string{"fs.watch", "fs.unwatch"} {
		raw, _ := json.Marshal(map[string]string{"path": outside})
		if _, err := r.handle(context.Background(), method, raw); err == nil {
			t.Fatalf("%s allowed outside path", method)
		}
	}
	path := filepath.Join(fx.agentCwd, "tracked.ts")
	raw, _ := json.Marshal(map[string]string{"path": path})
	if _, err := r.handle(context.Background(), "fs.watch", raw); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, path); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}
	r.pollFileChanges(context.Background())
	if count != 0 || len(r.fileWatches.paths) != 0 {
		t.Fatal("watch survived an escaping symlink swap")
	}
}

func TestHeadlessFileWatchLeaseRenewalExpiryAndMissingFile(t *testing.T) {
	fx := newGitFixture(t)
	r := registryWithCwds(t, fx.agentCwd)
	count := 0
	r.publish = func(string, json.RawMessage) { count++ }
	path := filepath.Join(fx.agentCwd, "not-yet-created.txt")
	raw, _ := json.Marshal(map[string]string{"path": path, "watchId": "browser-tab"})
	for i := 0; i < 3; i++ {
		if _, err := r.fsWatch(context.Background(), raw); err != nil {
			t.Fatal(err)
		}
	}
	entry := r.fileWatches.paths[path]
	if entry == nil || entry.refs != 0 || len(entry.leases) != 1 {
		t.Fatalf("renewal leaked references: %+v", entry)
	}
	if err := os.WriteFile(path, []byte("created"), 0600); err != nil {
		t.Fatal(err)
	}
	r.pollFileChanges(context.Background())
	if count != 1 {
		t.Fatalf("missing file creation events: %d", count)
	}
	if _, err := r.fsUnwatch(context.Background(), raw); err != nil {
		t.Fatal(err)
	}
	if len(r.fileWatches.paths) != 0 {
		t.Fatal("one unwatch did not release renewed lease")
	}
	if _, err := r.fsWatch(context.Background(), raw); err != nil {
		t.Fatal(err)
	}
	r.fileWatches.paths[path].leases["browser-tab"] = time.Now().Add(-time.Second)
	r.pollFileChanges(context.Background())
	if len(r.fileWatches.paths) != 0 {
		t.Fatal("abandoned lease did not expire")
	}
}
