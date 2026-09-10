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
	m.Entries[0].Name = "Reports/a.md"
	m.Entries = append(m.Entries, Entry{Name: "reports/b.md", Kind: "report", Size: 0, SHA256: Digest(nil)})
	if m.Validate() == nil {
		t.Fatal("accepted directory case alias")
	}
	m = fixtureManifest()
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
	if err := s.Materialize(dest); err != nil {
		t.Fatal("identical retry failed", err)
	}
	if err := os.WriteFile(filepath.Join(dest, "report.md"), []byte("user edit"), 0600); err != nil {
		t.Fatal(err)
	}
	if s.Materialize(dest) == nil {
		t.Fatal("retry overwrote changed artifact")
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
