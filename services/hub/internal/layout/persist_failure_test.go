package layout

// The layout document is the user's whole workspace arrangement, and layout.set
// answers with a bumped version and broadcasts layout.changed to every connected
// client (desktop, web, mobile) whether or not the bytes reached disk. Persisting
// is deliberately best-effort — a full/read-only state directory must not break
// the live sync — but a SILENT best-effort here means the arrangement reverts on
// the next hub start with nothing in any log, on any bus event, or in any UI.

import (
	"bytes"
	"encoding/json"
	"log"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
)

// captureLog redirects the standard logger for the duration of the test.
func captureLog(t *testing.T) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	prevOut, prevFlags := log.Writer(), log.Flags()
	log.SetOutput(&buf)
	log.SetFlags(0)
	t.Cleanup(func() {
		log.SetOutput(prevOut)
		log.SetFlags(prevFlags)
	})
	return &buf
}

func TestLayoutSetSaysSoWhenTheDocumentNeverReachedDisk(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("running as root: a 0500 directory is still writable")
	}
	dir := t.TempDir()
	path := filepath.Join(dir, "layout.json")
	b := broker.New()
	s := New(b, path)

	if _, err := s.Set(json.RawMessage(`{"data":{"panes":["a"]}}`)); err != nil {
		t.Fatalf("first set: %v", err)
	}
	first, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("first set did not persist: %v", err)
	}

	// A read-only state directory: ENOSPC / a full disk / wrong ownership after
	// a reinstall / a `serve` running as another uid all look like this.
	if err := os.Chmod(dir, 0o500); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(dir, 0o755) })

	buf := captureLog(t)
	if _, err := s.Set(json.RawMessage(`{"data":{"panes":["a","b","c"]}}`)); err != nil {
		t.Fatalf("second set: %v", err)
	}

	if err := os.Chmod(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, after) {
		t.Skip("the write unexpectedly succeeded; nothing to diagnose")
	}
	if got := buf.String(); !strings.Contains(got, "FAILED TO PERSIST") {
		t.Fatalf("layout.set reported success for a document that never reached disk and logged NOTHING.\nlog was: %q", got)
	}
}

func TestLayoutLoadSaysSoWhenAnExistingDocumentCannotBeRead(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "layout.json")
	if err := os.WriteFile(path, []byte("{not json"), 0o644); err != nil {
		t.Fatal(err)
	}

	buf := captureLog(t)
	s := New(broker.New(), path)

	doc, _ := s.Get(nil)
	if d, ok := doc.(Document); ok && d.Version != 0 {
		t.Fatalf("unparseable document was adopted: %+v", d)
	}
	if got := buf.String(); !strings.Contains(got, "FAILED TO PARSE") {
		t.Fatalf("an unreadable layout document was discarded silently — the next layout.set overwrites it.\nlog was: %q", got)
	}
}

func TestLayoutLoadIsQuietWhenThereIsSimplyNoFileYet(t *testing.T) {
	buf := captureLog(t)
	New(broker.New(), filepath.Join(t.TempDir(), "layout.json"))
	if got := buf.String(); got != "" {
		t.Fatalf("first run logged a failure for a file that simply does not exist yet: %q", got)
	}
}
