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

// THE PAIRING CREDENTIAL IS NOT REGENERABLE.
//
// loadOrCreateToken was shaped `read; on ENOENT, mint a fresh one` — right the
// first time, wrong every time after. A node whose <config>/workspacer survived
// but whose remote-token did not comes up printing a healthy ready banner and
// REFUSES every client, phone and federation peer that was paired against the
// old value. Nothing logs anything; the machine simply stops being reachable by
// anyone who already knew it.
//
// So a mint is allowed only where it cannot destroy a pairing: a config dir that
// holds nothing else, i.e. nobody has ever run here.
func TestLoadOrCreateTokenRefusesToSilentlyRemintOverExistingState(t *testing.T) {
	dir := t.TempDir()
	// An established install: the token is gone, the rest of the state is not.
	for _, f := range []string{"config.yaml", "tokens.json"} {
		if err := os.WriteFile(filepath.Join(dir, f), []byte("{}\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}

	tok, err := loadOrCreateToken(dir)
	if err == nil {
		t.Fatalf("minted a fresh pairing token (%q) over an established config dir with no "+
			"remote-token — every previously paired client/peer is now refused, silently", tok)
	}
	if _, statErr := os.Stat(filepath.Join(dir, "remote-token")); statErr == nil {
		t.Error("a refused mint must not leave a new token on disk — the old one may still be restorable")
	}
	for _, want := range []string{"remote-token", "--allow-new-token"} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("error %q does not mention %q — an operator reading this on a headless box "+
				"needs the path and the way forward", err, want)
		}
	}
}

// The other half: a genuinely virgin config dir must still mint, or first run
// is broken.
func TestLoadOrCreateTokenStillMintsOnAVirginConfigDir(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "workspacer") // does not exist yet
	tok, err := loadOrCreateToken(dir)
	if err != nil {
		t.Fatalf("first run must mint: %v", err)
	}
	if strings.TrimSpace(tok) == "" {
		t.Error("blank token on first run")
	}
}

// And the opt-in: an operator who knows the old token is gone for good says so
// once and gets a new identity.
func TestAllowNewTokenMintsOverExistingState(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "config.yaml"), []byte("{}\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	tok, err := loadOrCreateTokenAllowingNew(dir, true)
	if err != nil {
		t.Fatalf("--allow-new-token must mint: %v", err)
	}
	if strings.TrimSpace(tok) == "" {
		t.Error("blank token")
	}
}
