package main

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestDesktopNodeExecutable(t *testing.T) {
	dir := t.TempDir()
	executable := filepath.Join(dir, "brain.exe")
	if got := desktopNodeExecutable(executable); got != "node" {
		t.Fatalf("missing private runtime must use PATH, got %q", got)
	}
	candidate := filepath.Join(dir, "node.exe")
	if err := os.Mkdir(candidate, 0700); err != nil {
		t.Fatal(err)
	}
	if got := desktopNodeExecutable(executable); got != "node" {
		t.Fatalf("directory is not a runtime, got %q", got)
	}
	if err := os.Remove(candidate); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(candidate, []byte("fixture"), 0600); err != nil {
		t.Fatal(err)
	}
	want := "node"
	if runtime.GOOS == "windows" {
		want = candidate
	}
	if got := desktopNodeExecutable(executable); got != want {
		t.Fatalf("private runtime: got %q, want %q", got, want)
	}
	if got := desktopNodeExecutable(""); got != "node" {
		t.Fatalf("unavailable executable must use PATH, got %q", got)
	}
}
