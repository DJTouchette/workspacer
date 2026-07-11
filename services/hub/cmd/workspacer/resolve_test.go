package main

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// fakeBin drops an executable file named for the platform into dir.
func fakeBin(t *testing.T, dir, name string) string {
	t.Helper()
	p := filepath.Join(dir, exeName(name))
	if err := os.WriteFile(p, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestResolveBinOrder(t *testing.T) {
	sibling := t.TempDir()
	pathDir := t.TempDir()
	siblingHub := fakeBin(t, sibling, "hub")
	pathHub := fakeBin(t, pathDir, "hub")

	tests := []struct {
		name       string
		override   string
		siblingDir string
		path       string
		want       string
	}{
		{"explicit override wins over everything", "/explicit/hub", sibling, pathDir, "/explicit/hub"},
		{"sibling beats PATH", "", sibling, pathDir, siblingHub},
		{"PATH when no sibling", "", t.TempDir(), pathDir, pathHub},
		{"nothing found returns empty", "", t.TempDir(), t.TempDir(), ""},
		{"empty sibling dir degrades to PATH", "", "", pathDir, pathHub},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Setenv("PATH", tt.path)
			if got := resolveBin("hub", tt.override, tt.siblingDir); got != tt.want {
				t.Errorf("resolveBin = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestResolveBinIgnoresSiblingDirectory(t *testing.T) {
	// A directory named like the binary must not be picked up as the binary.
	sibling := t.TempDir()
	if err := os.Mkdir(filepath.Join(sibling, exeName("hub")), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", t.TempDir())
	if got := resolveBin("hub", "", sibling); got != "" {
		t.Errorf("resolveBin picked a directory: %q", got)
	}
}

func TestExeName(t *testing.T) {
	got := exeName("workspacer")
	if runtime.GOOS == "windows" {
		if got != "workspacer.exe" {
			t.Errorf("exeName = %q", got)
		}
	} else if got != "workspacer" {
		t.Errorf("exeName = %q", got)
	}
}

func TestDefaultWebappDir(t *testing.T) {
	dir := t.TempDir()
	// Nothing shipped → disabled, not a random guess.
	if got := defaultWebappDir(dir); got != "" {
		t.Fatalf("empty layout: got %q, want \"\"", got)
	}
	// Tarball layout: <dir>/web/index.html.
	sib := filepath.Join(dir, "web")
	mustWrite(t, filepath.Join(sib, "index.html"))
	if got := defaultWebappDir(dir); got != sib {
		t.Fatalf("tarball layout: got %q, want %q", got, sib)
	}
	// Packaged layout: resources/hub/<bin> next to resources/web — ../web wins
	// when <dir>/web is absent.
	res := t.TempDir()
	hubDir := filepath.Join(res, "hub")
	parent := filepath.Join(res, "web")
	mustWrite(t, filepath.Join(parent, "index.html"))
	if err := os.MkdirAll(hubDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if got := defaultWebappDir(hubDir); got != filepath.Join(hubDir, "..", "web") {
		t.Fatalf("packaged layout: got %q", got)
	}
	// A web/ dir WITHOUT index.html is not a web app.
	bare := t.TempDir()
	if err := os.MkdirAll(filepath.Join(bare, "web"), 0o755); err != nil {
		t.Fatal(err)
	}
	if got := defaultWebappDir(bare); got != "" {
		t.Fatalf("bare dir: got %q, want \"\"", got)
	}
}

func mustWrite(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
}
