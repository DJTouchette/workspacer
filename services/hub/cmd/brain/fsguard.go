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
// a live agent cwd or the config dir. Canonicalize means absolute with `..` AND
// symlinks resolved, so neither traversal nor a symlink planted inside a project
// can reach out of it.
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

// pathWithinRoots reports whether target canonicalizes to a location at or
// inside one of roots. Anything unverifiable is denied.
func pathWithinRoots(roots []string, target string) bool {
	ct, err := canonicalizePath(target)
	if err != nil {
		return false
	}
	for _, root := range roots {
		if root == "" {
			continue
		}
		cr, err := filepath.EvalSymlinks(root)
		if err != nil {
			abs, absErr := filepath.Abs(root)
			if absErr != nil {
				continue
			}
			cr = abs
		}
		if ct == cr || strings.HasPrefix(ct, cr+string(filepath.Separator)) {
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

// workspaceRoots is the allow-list for content-touching calls: live agent cwds
// plus the config dir (where library items, layouts and profiles live).
func (r *registry) workspaceRoots(ctx context.Context) []string {
	return append(r.agentCwds(ctx), configDir())
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

// assertPathAllowed rejects a call whose path escapes the allowed roots. The
// message deliberately does not echo the resolved path — it goes to a remote
// caller, and confirming where a denied path landed is a probe primitive.
func assertPathAllowed(capability, target string, roots []string) error {
	if !pathWithinRoots(roots, target) {
		return fmt.Errorf("%s: path is outside the allowed workspace (agent cwds + config dir)", capability)
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
