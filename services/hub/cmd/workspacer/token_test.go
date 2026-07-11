package main

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestLoadOrCreateToken(t *testing.T) {
	t.Run("mints, persists 0600, and reuses", func(t *testing.T) {
		dir := t.TempDir()
		tok, err := loadOrCreateToken(dir)
		if err != nil {
			t.Fatal(err)
		}
		if len(tok) < 20 {
			t.Errorf("token suspiciously short: %q", tok)
		}
		if runtime.GOOS != "windows" {
			st, err := os.Stat(filepath.Join(dir, "remote-token"))
			if err != nil {
				t.Fatal(err)
			}
			// A bearer secret must be owner-only, like the desktop writes it.
			if st.Mode().Perm() != 0o600 {
				t.Errorf("token file mode = %v, want 0600", st.Mode().Perm())
			}
		}
		again, err := loadOrCreateToken(dir)
		if err != nil {
			t.Fatal(err)
		}
		if again != tok {
			t.Errorf("second load minted a new token (%q != %q) — paired clients would break", again, tok)
		}
	})

	t.Run("reads a desktop-written token verbatim", func(t *testing.T) {
		dir := t.TempDir()
		// The desktop writes the raw token; tolerate trailing whitespace.
		if err := os.WriteFile(filepath.Join(dir, "remote-token"), []byte("desktop-tok\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		tok, err := loadOrCreateToken(dir)
		if err != nil {
			t.Fatal(err)
		}
		if tok != "desktop-tok" {
			t.Errorf("token = %q, want the desktop's", tok)
		}
	})

	t.Run("empty file is treated as missing", func(t *testing.T) {
		dir := t.TempDir()
		if err := os.WriteFile(filepath.Join(dir, "remote-token"), []byte("  \n"), 0o600); err != nil {
			t.Fatal(err)
		}
		tok, err := loadOrCreateToken(dir)
		if err != nil {
			t.Fatal(err)
		}
		if strings.TrimSpace(tok) == "" {
			t.Error("blank token returned from an empty file")
		}
	})
}

func TestConfigDirRespectsXDG(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("XDG is a unix convention")
	}
	t.Setenv("XDG_CONFIG_HOME", "/tmp/xdg-test")
	if got := configDir(); got != "/tmp/xdg-test/workspacer" {
		t.Errorf("configDir = %q", got)
	}
}
