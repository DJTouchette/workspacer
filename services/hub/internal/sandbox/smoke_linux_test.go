package sandbox

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

// A real bubblewrap confinement check (Linux + bwrap only): a process confined
// to its plugin dir can write inside it but NOT outside it. Verifies the actual
// argv buildBwrapArgs produces enforces, not just its shape.
func TestBwrapReallyConfinesWrites(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("linux only")
	}
	if _, err := exec.LookPath("bwrap"); err != nil {
		t.Skip("bwrap not installed")
	}
	root := t.TempDir()    // the plugin dir (writable)
	outside := t.TempDir() // somewhere the sidecar must NOT write
	target := filepath.Join(outside, "escape.txt")

	run := func(path string) error {
		res := Wrap("/bin/sh", []string{"-c", "echo pwned > " + path}, Policy{WriteRoots: []string{root}})
		if !res.Available {
			t.Skip("bwrap wrap unavailable")
		}
		return exec.Command(res.Path, res.Args...).Run()
	}

	// Writing inside the granted root succeeds.
	inside := filepath.Join(root, "ok.txt")
	if err := run(inside); err != nil {
		t.Fatalf("write inside root should succeed: %v", err)
	}
	if _, err := os.Stat(inside); err != nil {
		t.Fatalf("inside file not created: %v", err)
	}

	// Writing outside every write root must fail, and must not create the file.
	if err := run(target); err == nil {
		t.Error("write outside the root should have been denied by bwrap")
	}
	if _, err := os.Stat(target); err == nil {
		t.Errorf("escape file was created at %s — confinement breached", target)
	}
}
