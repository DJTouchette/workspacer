package main

import (
	"crypto/rand"
	"encoding/base64"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/statelost"
)

// configDir mirrors the desktop app's getConfigDir (configService.ts):
// %APPDATA%\workspacer on Windows, $XDG_CONFIG_HOME/workspacer or
// ~/.config/workspacer elsewhere. Sharing the directory is deliberate — the
// CLI and the desktop app must agree on where the pairing token lives. The
// single definition lives in internal/authtoken so the hub's tokens.json
// default resolves to the very same directory.
func configDir() string {
	return authtoken.ConfigDir()
}

// loadOrCreateToken returns the hub bus token, minting + persisting one on
// first use. The hub requires no token by default (loopback trust), but a
// headless server needs one: it's the pairing credential for /remote, /m and
// the bus, and the basis of plugin capability scoping. We reuse the exact file
// the desktop app persists (<config>/remote-token) so a phone paired against
// the desktop keeps working against `workspacer serve` and vice versa.
func loadOrCreateToken(dir string) (string, error) {
	return loadOrCreateTokenAllowingNew(dir, false)
}

// loadOrCreateTokenAllowingNew is loadOrCreateToken with the first-run mint made
// explicit.
//
// MINTING A FRESH TOKEN IS NOT A RECOVERY, IT IS A NEW IDENTITY. The token is
// the pairing credential: the bearer secret on /bus, /remote and /m, the basis
// of plugin capability scoping, and the value a federating hub presents to reach
// this machine. Re-minting it does not restore service — it revokes every
// existing pairing at once — and the process that did it goes on to print a
// perfectly healthy ready banner. A phone that stops working, a peer stuck on
// hub.peer.disconnected, and a node that was never provisioned all look
// identical from the outside.
//
// So the mint is allowed only where it cannot destroy a pairing: a config dir
// that holds nothing else, meaning nobody has ever run here. When the directory
// still holds the rest of the state and only this file is gone, that is loss,
// and `serve` REFUSES TO START rather than come up as somebody else. Refusing is
// affordable here in a way it is not elsewhere in this codebase: `serve` is a
// foreground CLI whose exit is the loudest signal available, there is no useful
// work a mis-identified node can do anyway, and the operator has two exits that
// are one word long — pass the real token with --token/$HUB_TOKEN, or accept the
// new identity with --allow-new-token.
//
// (The desktop's twin, hubDaemon.ts loadOrCreateToken, carries the same
// detection but only warns: an Electron app that refuses to boot over this
// leaves the user no way to see the message.)
func loadOrCreateTokenAllowingNew(dir string, allowNew bool) (string, error) {
	file := filepath.Join(dir, "remote-token")
	if b, err := os.ReadFile(file); err == nil {
		if tok := strings.TrimSpace(string(b)); tok != "" {
			return tok, nil
		}
	}
	if !allowNew && statelost.Suspected(dir, "remote-token") {
		return "", fmt.Errorf(
			"STATE LOSS: %s is missing, but %s still holds the rest of this install.\n"+
				"  That file is the pairing credential — minting a new one would silently refuse every client,\n"+
				"  phone and federation peer paired against the old value, while the server looked healthy.\n"+
				"  Restore it from your backup or volume, or pass the real token with --token / $HUB_TOKEN.\n"+
				"  If it is gone for good, re-pair everything and start with --allow-new-token (or\n"+
				"  $WORKSPACER_ALLOW_NEW_TOKEN=1)", file, dir)
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
