package main

import (
	"crypto/rand"
	"encoding/base64"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// configDir mirrors the desktop app's getConfigDir (configService.ts):
// %APPDATA%\workspacer on Windows, $XDG_CONFIG_HOME/workspacer or
// ~/.config/workspacer elsewhere. Sharing the directory is deliberate — the
// CLI and the desktop app must agree on where the pairing token lives.
func configDir() string {
	if runtime.GOOS == "windows" {
		if appData := os.Getenv("APPDATA"); appData != "" {
			return filepath.Join(appData, "workspacer")
		}
		home, _ := os.UserHomeDir()
		return filepath.Join(home, "AppData", "Roaming", "workspacer")
	}
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "workspacer")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".config", "workspacer")
}

// loadOrCreateToken returns the hub bus token, minting + persisting one on
// first use. The hub requires no token by default (loopback trust), but a
// headless server needs one: it's the pairing credential for /remote, /m and
// the bus, and the basis of plugin capability scoping. We reuse the exact file
// the desktop app persists (<config>/remote-token) so a phone paired against
// the desktop keeps working against `workspacer serve` and vice versa.
func loadOrCreateToken(dir string) (string, error) {
	file := filepath.Join(dir, "remote-token")
	if b, err := os.ReadFile(file); err == nil {
		if tok := strings.TrimSpace(string(b)); tok != "" {
			return tok, nil
		}
	}
	// 24 random bytes, base64url — the same shape the desktop generates.
	raw := make([]byte, 24)
	if _, err := rand.Read(raw); err != nil {
		return "", err
	}
	tok := base64.RawURLEncoding.EncodeToString(raw)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	// 0600: the token is a bearer secret — owner-only, like an SSH key.
	if err := os.WriteFile(file, []byte(tok), 0o600); err != nil {
		return "", err
	}
	return tok, nil
}
