package bus

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
)

// Filesystem path confinement for plugin capability grants.
//
// A plugin granted a path-scoped capability (fs.read, fs.write, search.project,
// …) may only touch paths inside the roots its grant declares. The trick that
// makes this safe is *canonicalize then contain*: resolve `..` and symlinks on
// both the target and the roots before the prefix check, so neither directory
// traversal nor a symlink pointing out of the root can escape. A purely textual
// prefix check would be fooled by either.

// canonicalize returns the absolute, symlink-resolved form of path. For a target
// that doesn't exist yet (e.g. a file fs.write is about to create), it resolves
// the longest existing ancestor and re-appends the non-existent remainder — so a
// write can't be aimed outside a root through a not-yet-created intermediate, and
// a symlink anywhere along the existing prefix is still followed. Fails closed:
// any error other than "doesn't exist" (e.g. a permission error mid-walk) is
// returned, never silently treated as contained.
func canonicalize(path string) (string, error) {
	abs, err := filepath.Abs(filepath.Clean(path))
	if err != nil {
		return "", err
	}
	rem := ""
	cur := abs
	for {
		real, err := filepath.EvalSymlinks(cur)
		if err == nil {
			if rem == "" {
				return real, nil
			}
			return filepath.Join(real, rem), nil
		}
		if !os.IsNotExist(err) {
			return "", err // permission/other — fail closed
		}
		parent := filepath.Dir(cur)
		if parent == cur {
			// Walked to the filesystem root without finding an existing ancestor
			// (only plausible for an unusual abs path). Use the cleaned form.
			return abs, nil
		}
		rem = filepath.Join(filepath.Base(cur), rem)
		cur = parent
	}
}

// within reports whether canonical target sits at or inside canonical root. The
// separator guard is what stops a sibling whose name shares a prefix — `/srv/foo`
// must not be considered inside root `/srv/fo`.
func within(root, target string) bool {
	if target == root {
		return true
	}
	return strings.HasPrefix(target, root+string(os.PathSeparator))
}

// pathWithinRoots reports whether target resolves to a location inside one of the
// (already-canonical) roots. The target is canonicalized here; roots are
// canonicalized once at grant registration so the hot path doesn't re-walk them.
func pathWithinRoots(roots []string, target string) (bool, error) {
	ct, err := canonicalize(target)
	if err != nil {
		return false, err
	}
	for _, r := range roots {
		if within(r, ct) {
			return true, nil
		}
	}
	return false, nil
}

// paramString pulls a string field out of a call's JSON params. ok is false when
// params don't parse, the field is absent, or it isn't a non-empty string — all
// of which the caller treats as "can't verify the path" → deny.
func paramString(params json.RawMessage, field string) (value string, ok bool) {
	if len(params) == 0 {
		return "", false
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(params, &m); err != nil {
		return "", false
	}
	raw, present := m[field]
	if !present {
		return "", false
	}
	var s string
	if err := json.Unmarshal(raw, &s); err != nil {
		return "", false
	}
	return s, s != ""
}
