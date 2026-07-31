// Filesystem path confinement for the brain's fs.* / search.project handlers
// (SECURITY.md #8).
//
// This is a port of the confinement in the desktop's hubCapabilities.ts, and it
// has to exist HERE as well because of who actually answers the bus: the desktop
// registers those methods through `cat(...)`, which is a no-op whenever the
// catalog is delegated to this brain — the default. So the guard in the app was
// unreachable in normal operation and these handlers were serving arbitrary host
// paths to any bus client: a remote-share client on the tailnet, a plugin, or an
// agent through the MCP facade. Same rule, same failure mode, both providers.
//
// The rule: a caller-supplied path must canonicalize to a location at or inside
// a live agent cwd or one of the config-dir stores (library/, layouts/,
// sessions/), must not be a credential file by name, and must not land anywhere
// else in the config dir even when an agent cwd contains it. Canonicalize means
// absolute with `..` AND symlinks resolved, so neither traversal nor a symlink
// planted inside a project can reach out of it.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// canonicalizePath returns p as an absolute path with `..` and symlinks
// resolved. For a target that does not exist yet (the file fs.write is about to
// create) it resolves the longest existing ancestor and re-appends the missing
// tail — so a write cannot be aimed outside a root through a not-yet-created
// intermediate, while a symlink along the existing prefix is still followed.
// Any non-ENOENT error fails closed.
func canonicalizePath(p string) (string, error) {
	abs, err := filepath.Abs(p)
	if err != nil {
		return "", err
	}
	rem := ""
	for {
		real, err := filepath.EvalSymlinks(abs)
		if err == nil {
			if rem == "" {
				return real, nil
			}
			return filepath.Join(real, rem), nil
		}
		if !os.IsNotExist(err) {
			return "", err // permission, ELOOP, … → fail closed
		}
		parent := filepath.Dir(abs)
		if parent == abs { // reached the filesystem root
			if rem == "" {
				return abs, nil
			}
			return filepath.Join(abs, rem), nil
		}
		if rem == "" {
			rem = filepath.Base(abs)
		} else {
			rem = filepath.Join(filepath.Base(abs), rem)
		}
		abs = parent
	}
}

// canonicalRoot is the comparable form of an allowed root: its realpath when it
// exists, a plain absolute path when it does not — the config stores are created
// lazily and must still be comparable before anything has been saved.
func canonicalRoot(root string) (string, bool) {
	if cr, err := filepath.EvalSymlinks(root); err == nil {
		return cr, true
	}
	abs, err := filepath.Abs(root)
	if err != nil {
		return "", false
	}
	return abs, true
}

// isWithin reports whether an ALREADY canonicalized target sits at or inside
// root. Callers that hold the canonical form use this directly so a path is
// resolved once per check rather than once per root.
func isWithin(canonicalTarget, root string) bool {
	if root == "" {
		return false
	}
	cr, ok := canonicalRoot(root)
	if !ok {
		return false
	}
	return canonicalTarget == cr || strings.HasPrefix(canonicalTarget, cr+string(filepath.Separator))
}

// pathWithinRoots reports whether target canonicalizes to a location at or
// inside one of roots. Anything unverifiable is denied.
func pathWithinRoots(roots []string, target string) bool {
	ct, err := canonicalizePath(target)
	if err != nil {
		return false
	}
	for _, root := range roots {
		if isWithin(ct, root) {
			return true
		}
	}
	return false
}

// Live agent cwds change as agents spawn and stop, and in catalog scope reading
// them means an HTTP round trip to claudemon. A short TTL keeps an interactive
// file tree from paying that per entry while still picking up a new agent's cwd
// almost immediately. Deny decisions are never cached — only the root list is.
const cwdCacheTTL = 2 * time.Second

var (
	cwdCacheMu   sync.Mutex
	cwdCacheAt   time.Time
	cwdCacheVals []string
)

// agentCwds returns the cwd of every session claudemon knows about (or the local
// store's, in full scope).
func (r *registry) agentCwds(ctx context.Context) []string {
	cwdCacheMu.Lock()
	if !cwdCacheAt.IsZero() && time.Since(cwdCacheAt) < cwdCacheTTL {
		cached := cwdCacheVals
		cwdCacheMu.Unlock()
		return cached
	}
	cwdCacheMu.Unlock()

	var raws []json.RawMessage
	if r.store != nil {
		raws = r.store.all()
	} else if r.cm != nil {
		if listed, err := r.cm.listSessions(ctx); err == nil {
			// /sessions is an array of session objects; a shape change here must
			// not silently widen the allow-list, so a decode failure yields none.
			_ = json.Unmarshal(listed, &raws)
		}
	}

	cwds := make([]string, 0, len(raws))
	for _, raw := range raws {
		var s struct {
			Cwd string `json:"cwd"`
		}
		if err := json.Unmarshal(raw, &s); err == nil && s.Cwd != "" {
			cwds = append(cwds, s.Cwd)
		}
	}

	cwdCacheMu.Lock()
	cwdCacheAt = time.Now()
	cwdCacheVals = cwds
	cwdCacheMu.Unlock()
	return cwds
}

// configStoreRoots are the config-dir subtrees a client legitimately edits
// through fs.*: library items, layout templates and saved sessions.
//
// The config DIR itself used to be the root, which was far too wide: it also
// holds remote-token and tokens.json (bus credentials), remote-server.json,
// vapid.json, the Electron cookie/localStorage jars, and every installed
// plugin's .bus-token and plaintext .settings.json. A plugin whose manifest
// declared an absolute fs.read path landing in there could read remote-token and
// reconnect as a TRUSTED bus connection — which unlocks /plugins/install, i.e.
// arbitrary commands. Everything else in the config dir has a dedicated
// capability (config.*, claude.profiles.*) and never needed raw file access.
func configStoreRoots() []string {
	base := configDir()
	return []string{
		filepath.Join(base, "library"),
		filepath.Join(base, "layouts"),
		filepath.Join(base, "sessions"),
	}
}

// workspaceRoots is the allow-list for content-touching calls: live agent cwds
// plus the config-dir stores above.
func (r *registry) workspaceRoots(ctx context.Context) []string {
	cwds := r.agentCwds(ctx)
	stores := configStoreRoots()
	roots := make([]string, 0, len(cwds)+len(stores))
	roots = append(roots, cwds...)
	return append(roots, stores...)
}

// browseRoots is the wider allow-list for the directory picker (fs.listDir):
// the home tree, so a user can navigate to a project before an agent runs in it,
// but still not `/etc` or another user's home.
func (r *registry) browseRoots(ctx context.Context) []string {
	home, err := os.UserHomeDir()
	if err != nil {
		return r.workspaceRoots(ctx)
	}
	return append(r.workspaceRoots(ctx), home)
}

// secretBasenames are credential files by name, denied wherever they resolve.
// The roots above already keep the config dir's plugin tree out of reach, but a
// root is only as narrow as the cwds an agent runs in: spawn an agent in
// ~/.config/workspacer (or run `workspacer plugin dev` on a directory inside a
// project) and these come back into an allowed root. A bus token is a bus token
// wherever it sits, so deny it by name too.
var secretBasenames = map[string]bool{
	".bus-token":     true, // per-plugin bus credential
	".settings.json": true, // per-plugin settings, secrets in plaintext
}

// pathIsSecret is the second gate, applied to every guarded path after the roots
// check — reads AND writes, because handing a token out is a privilege promotion
// and overwriting one is a denial of service on the whole bus.
//
// Narrowing the config root is not enough on its own: an agent cwd is a root
// too, so spawning an agent in $HOME (or in ~/.config) re-admits the entire
// config dir through THAT root. So anything landing in the config dir outside
// library/ layouts/ sessions/ is refused here regardless of which root allowed
// it. Enumerating the credentials by name was the earlier shape and it was too
// narrow — it left config.yaml writable (updates.channel is concatenated into
// the electron-updater feed URL, so that is remote code execution laundered
// through the update dialog), and left workspacer.db and the legacy plaintext
// plugin-settings.json readable. This deny-the-whole-dir rule is the one
// hubCapabilities.ts isSecretPath implements; the two must stay word for word.
func pathIsSecret(target string) bool {
	ct, err := canonicalizePath(target)
	if err != nil {
		return true // unverifiable → deny, same posture as pathWithinRoots
	}
	if secretBasenames[filepath.Base(ct)] {
		return true
	}
	if !isWithin(ct, configDir()) {
		return false
	}
	for _, root := range configStoreRoots() {
		if isWithin(ct, root) {
			return false
		}
	}
	return true
}

// assertPathAllowed rejects a call whose path escapes the allowed roots or lands
// on a credential file. The message deliberately does not echo the resolved path
// and is the same for both refusals — it goes to a remote caller, and confirming
// where a denied path landed (or that it hit something worth protecting) is a
// probe primitive.
func assertPathAllowed(capability, target string, roots []string) error {
	if !pathWithinRoots(roots, target) || pathIsSecret(target) {
		return fmt.Errorf("%s: path is outside the allowed workspace (agent cwds + config stores)", capability)
	}
	return nil
}

// resetCwdCacheForTest drops the memoized root list. Tests change what counts as
// a live cwd between cases, and would otherwise see a stale allow-list.
func resetCwdCacheForTest() {
	cwdCacheMu.Lock()
	cwdCacheAt = time.Time{}
	cwdCacheVals = nil
	cwdCacheMu.Unlock()
}
