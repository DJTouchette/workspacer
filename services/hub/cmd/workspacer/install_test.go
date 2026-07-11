package main

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestPickInstallDir(t *testing.T) {
	writableDirs := func(dirs ...string) func(string) bool {
		return func(d string) bool {
			for _, w := range dirs {
				if d == w {
					return true
				}
			}
			return false
		}
	}
	tests := []struct {
		name         string
		goos         string
		home         string
		localAppData string
		writable     func(string) bool
		want         string
	}{
		{
			name: "unix prefers /usr/local/bin when writable",
			goos: "linux", home: "/home/u",
			writable: writableDirs("/usr/local/bin"),
			want:     "/usr/local/bin",
		},
		{
			name: "unix falls back to ~/.local/bin without root",
			goos: "linux", home: "/home/u",
			writable: writableDirs(),
			want:     filepath.Join("/home/u", ".local", "bin"),
		},
		{
			name: "darwin uses the same unix order",
			goos: "darwin", home: "/Users/u",
			writable: writableDirs("/usr/local/bin"),
			want:     "/usr/local/bin",
		},
		{
			name: "windows uses LOCALAPPDATA",
			goos: "windows", home: `C:\Users\u`, localAppData: `C:\Users\u\AppData\Local`,
			writable: writableDirs(),
			want:     filepath.Join(`C:\Users\u\AppData\Local`, "workspacer", "bin"),
		},
		{
			name: "windows derives LOCALAPPDATA from home when unset",
			goos: "windows", home: `C:\Users\u`,
			writable: writableDirs(),
			want:     filepath.Join(`C:\Users\u`, "AppData", "Local", "workspacer", "bin"),
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := pickInstallDir(tt.goos, tt.home, tt.localAppData, tt.writable)
			if got != tt.want {
				t.Errorf("pickInstallDir = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestInstallBinary(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("symlink semantics are unix-specific")
	}
	src := filepath.Join(t.TempDir(), "workspacer")
	if err := os.WriteFile(src, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}

	t.Run("creates a symlink to the real binary", func(t *testing.T) {
		dst := filepath.Join(t.TempDir(), "workspacer")
		if err := installBinary(src, dst); err != nil {
			t.Fatal(err)
		}
		link, err := os.Readlink(dst)
		if err != nil {
			t.Fatalf("expected a symlink at %s: %v", dst, err)
		}
		if link != src {
			t.Errorf("symlink -> %q, want %q", link, src)
		}
	})

	t.Run("replaces an existing entry idempotently", func(t *testing.T) {
		dst := filepath.Join(t.TempDir(), "workspacer")
		if err := os.WriteFile(dst, []byte("old"), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := installBinary(src, dst); err != nil {
			t.Fatal(err)
		}
		if _, err := os.Readlink(dst); err != nil {
			t.Errorf("old file was not replaced by the link: %v", err)
		}
		// Second run over our own link must also succeed (re-install).
		if err := installBinary(src, dst); err != nil {
			t.Errorf("re-install failed: %v", err)
		}
	})

	t.Run("installing onto itself is a no-op", func(t *testing.T) {
		if err := installBinary(src, src); err != nil {
			t.Fatal(err)
		}
		if _, err := os.Readlink(src); err == nil {
			t.Error("self-install replaced the real binary with a link to itself")
		}
	})
}

func TestOnPath(t *testing.T) {
	sep := string(os.PathListSeparator)
	path := strings.Join([]string{"/usr/bin", "/home/u/.local/bin"}, sep)
	tests := []struct {
		dir  string
		want bool
	}{
		{"/home/u/.local/bin", true},
		{"/usr/bin", true},
		{"/usr/local/bin", false},
		{"", false},
	}
	for _, tt := range tests {
		if got := onPath(tt.dir, path); got != tt.want {
			t.Errorf("onPath(%q) = %v, want %v", tt.dir, got, tt.want)
		}
	}
}
