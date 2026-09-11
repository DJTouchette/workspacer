package taskartifacts

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func fixtureManifest() Manifest {
	return Manifest{Version: Version, Task: strings.Repeat("a", 32), Origin: strings.Repeat("b", 32), Producer: strings.Repeat("c", 32), Commit: strings.Repeat("d", 40), ObjectFormat: "sha1", Entries: []Entry{{Name: "report.md", Kind: "report", Size: 5, SHA256: Digest([]byte("hello"))}}}
}

func TestPortableManifestRefusesAliasesAndLimits(t *testing.T) {
	for _, name := range []string{"../secret", "/absolute", `C:\secret`, `.git/config`, "a/.git/config", "NUL.md", "COM1.png", "a/../b", "foo.", "foo ", "é.md", "a:stream", "a\\b", "a//b"} {
		t.Run(name, func(t *testing.T) {
			m := fixtureManifest()
			m.Entries[0].Name = name
			if m.Validate() == nil {
				t.Fatal("accepted unsafe path")
			}
		})
	}
	for _, name := range []string{"REPORT.md", "report.md/child"} {
		m := fixtureManifest()
		e := m.Entries[0]
		e.Name = name
		m.Entries = append(m.Entries, e)
		if m.Validate() == nil {
			t.Fatal("accepted colliding paths")
		}
	}
	m := fixtureManifest()
	m.Entries[0].Size = FileBytes + 1
	if m.Validate() == nil {
		t.Fatal("accepted excessive size")
	}
}

func TestChunkRetryAndRequiredCustody(t *testing.T) {
	m := fixtureManifest()
	s, err := Open(t.TempDir(), m)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	if s.Verify() == nil {
		t.Fatal("missing bytes ready")
	}
	if err := s.Write(0, 0, []byte("hel")); err != nil {
		t.Fatal(err)
	}
	if err := s.Write(0, 0, []byte("hel")); err != nil {
		t.Fatal(err)
	}
	if s.Write(0, 0, []byte("bad")) == nil {
		t.Fatal("conflicting retry accepted")
	}
	if s.Write(1, 0, nil) == nil {
		t.Fatal("unselected object accessible")
	}
	if err := s.Write(0, 3, []byte("lo")); err != nil {
		t.Fatal(err)
	}
	if err := s.Verify(); err != nil {
		t.Fatal(err)
	}
	dest := filepath.Join(t.TempDir(), ".workspacer", "handoffs", m.Task)
	if err := s.Materialize(dest); err != nil {
		t.Fatal(err)
	}
	b, err := os.ReadFile(filepath.Join(dest, "report.md"))
	if err != nil || string(b) != "hello" {
		t.Fatal("custody mismatch", err)
	}
}

func TestManifestSealBindsOwnershipAndCommit(t *testing.T) {
	m := fixtureManifest()
	a, err := m.Seal()
	if err != nil {
		t.Fatal(err)
	}
	m.Origin = strings.Repeat("e", 32)
	b, _ := m.Seal()
	if a == b {
		t.Fatal("origin not bound")
	}
	m.Commit = strings.Repeat("f", 40)
	c, _ := m.Seal()
	if b == c {
		t.Fatal("commit not bound")
	}
}

func TestInterruptedChunkAndMaterializationRetry(t *testing.T) {
	m := fixtureManifest()
	dir := t.TempDir()
	// Simulate a process dying midway through an append, before its ACK.
	if err := os.WriteFile(filepath.Join(dir, "0.bytes"), []byte("he"), 0600); err != nil {
		t.Fatal(err)
	}
	s, err := Open(dir, m)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	if err := s.Write(0, 0, []byte("hello")); err != nil {
		t.Fatal(err)
	}
	dest := t.TempDir()
	if err := s.Materialize(dest); err != nil {
		t.Fatal(err)
	}
	if err := s.Materialize(dest); err != nil {
		t.Fatal("lost finalize ACK was not idempotent", err)
	}
	if err := os.WriteFile(filepath.Join(dest, "report.md"), []byte("changed"), 0600); err != nil {
		t.Fatal(err)
	}
	if s.Materialize(dest) == nil {
		t.Fatal("overwrote unexpected materialized bytes")
	}
}

func TestStagingRejectsLinks(t *testing.T) {
	m := fixtureManifest()
	for _, hard := range []bool{false, true} {
		dir := t.TempDir()
		outside := filepath.Join(t.TempDir(), "private.md")
		if err := os.WriteFile(outside, []byte("hello"), 0600); err != nil {
			t.Fatal(err)
		}
		var err error
		if hard {
			err = os.Link(outside, filepath.Join(dir, "0.bytes"))
		} else {
			err = os.Symlink(outside, filepath.Join(dir, "0.bytes"))
		}
		if err != nil {
			t.Log("filesystem does not support fixture link", err)
			continue
		}
		s, err := Open(dir, m)
		if err != nil {
			t.Fatal(err)
		}
		if s.Verify() == nil {
			t.Fatal("linked bytes admitted")
		}
		if s.Write(0, 0, []byte("wrong")) == nil {
			t.Fatal("linked bytes overwritten")
		}
		s.Close()
		got, _ := os.ReadFile(outside)
		if string(got) != "hello" {
			t.Fatal("outside bytes changed")
		}
	}
}

func TestReportImagesStayInsideSelectedManifest(t *testing.T) {
	entries := []Entry{{Name: "images/diagram.png", Kind: "image"}}
	if err := validateReportImages("notes/scout.md", []byte("![diagram](../images/diagram.png)\r\n"), entries); err != nil {
		t.Fatal(err)
	}
	for _, content := range []string{"![x](https://example.test/x.png)", "![x](/producer/absolute.png)", "![x](../../outside.png)", "![x](missing.png)", "![x][reference]", "<img src=\"https://example.test/x.png\">"} {
		if validateReportImages("scout.md", []byte(content), entries) == nil {
			t.Fatalf("accepted nonportable image: %s", content)
		}
	}
}
