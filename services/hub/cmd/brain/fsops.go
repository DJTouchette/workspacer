package main

// Host filesystem capabilities — the headless equivalent of the app's fs.* /
// app.* handlers (fileService.ts). Pure Go: a web/TUI client browses the host to
// choose a working directory and reads/writes/lists host files for an editor
// pane, identically with or without the GUI.

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"
)

const maxReadBytes = 5 * 1024 * 1024

// listDirResult mirrors the app's fs.listDir shape: directories only (you spawn
// an agent *in* a folder), hidden entries skipped.
type listDirResult struct {
	Path   string   `json:"path"`
	Parent string   `json:"parent"`
	Home   string   `json:"home"`
	Dirs   []string `json:"dirs"`
}

// listHostDir lists the directory NAMES one level under `p`.
//
// `p` is the string assertPathAllowed returned and it is used VERBATIM. Every
// helper in this file used to re-normalize its argument — TrimSpace, then
// expandTilde, then filepath.Abs (which Cleans) — which is BINDING DECISION 2
// inverted: the guard validated one string and the handler opened another. The
// trailing space was the whole exploit. "<root>/.. " is an ordinary component
// named `.. ` and canonicalizes to a path INSIDE the root, so the guard allows
// it; TrimSpace then turned it back into "<root>/.." and Abs collapsed that to
// the root's parent, so `fs.listDir {"path": "$HOME/.. "}` enumerated /home and
// "<configDir>/layouts/.. " enumerated the config dir itself. The same trim
// re-attached a trailing space to a symlink name the guard had just refused
// ("<root>/link " checked, "<root>/link" opened). Nothing here may touch the
// string: it is already absolute, already '..'-free and already symlink-
// resolved, so any further normalization can only make it name something else.
//
// The empty-path default (the picker opening on $HOME) lives in the fs.listDir
// handler, BEFORE the guard, because an empty path has to become a real one
// while there is still a decision to make.
func listHostDir(p string) (listDirResult, error) {
	home, _ := os.UserHomeDir()
	if p == "" {
		return listDirResult{}, fmt.Errorf("listHostDir requires the guarded canonical path")
	}
	resolved := p
	entries, err := os.ReadDir(resolved)
	if err != nil {
		return listDirResult{}, err
	}
	dirs := []string{}
	for _, e := range entries {
		if e.IsDir() && !strings.HasPrefix(e.Name(), ".") {
			dirs = append(dirs, e.Name())
		}
	}
	sort.Strings(dirs)
	return listDirResult{Path: resolved, Parent: filepath.Dir(resolved), Home: home, Dirs: dirs}, nil
}

type readFileResult struct {
	Path     string `json:"path"`
	Contents string `json:"contents"`
	Size     int64  `json:"size"`
}

// readTextFile ports fileService.readTextFile: regular-file/size/binary/utf-8
// guards, so we never hand back (or later clobber) a binary or lossy file.
//
// `p` is the guard's canonical path and is opened verbatim (see listHostDir).
func readTextFile(p string) (*readFileResult, error) {
	full := p
	st, err := os.Stat(full)
	if err != nil {
		return nil, err
	}
	if !st.Mode().IsRegular() {
		return nil, fmt.Errorf("not a regular file: %s", p)
	}
	if st.Size() > maxReadBytes {
		return nil, fmt.Errorf("file is %d bytes (max %d)", st.Size(), maxReadBytes)
	}
	buf, err := os.ReadFile(full)
	if err != nil {
		return nil, err
	}
	if bytes.IndexByte(buf, 0) >= 0 {
		return nil, fmt.Errorf("file appears to be binary")
	}
	if !utf8.Valid(buf) {
		return nil, fmt.Errorf("file is not valid UTF-8")
	}
	return &readFileResult{Path: p, Contents: string(buf), Size: st.Size()}, nil
}

// writeHostFile mirrors fileService.writeTextFile, including creating missing
// parents: callers (fs.write, and plugins storing data under
// <project>/.workspacer/plugins/<id>/) rely on that, and the desktop twin has
// always done it. Containment is checked by the handler BEFORE this runs, so any
// directory created here is inside an allowed root by construction — which only
// holds because `p` is the guard's canonical path, written verbatim (see
// listHostDir).
func writeHostFile(p, contents string) error {
	full := p
	if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
		return err
	}
	return os.WriteFile(full, []byte(contents), 0o644)
}

// dirEntry / listEntriesResult mirror fileService.listDir (the file-tree list).
type dirEntry struct {
	Name  string `json:"name"`
	Path  string `json:"path"`
	IsDir bool   `json:"isDir"`
}
type listEntriesResult struct {
	Path    string     `json:"path"`
	Entries []dirEntry `json:"entries"`
}

// listEntries lists one directory level for an editor file tree, hiding `.git`
// and (inside a repo) anything `.gitignore`d — using git's own logic via
// `git check-ignore`, so nested ignore files are honoured. Ports
// fileService.listDir.
//
// `dirPath` is the guard's canonical path and is listed verbatim (see
// listHostDir).
func listEntries(dirPath string) (listEntriesResult, error) {
	resolved := dirPath
	dirents, err := os.ReadDir(resolved)
	if err != nil {
		return listEntriesResult{}, err
	}
	var names []string
	for _, e := range dirents {
		if e.Name() != ".git" {
			names = append(names, e.Name())
		}
	}
	ignored := gitIgnored(resolved, names)

	entries := []dirEntry{}
	for _, e := range dirents {
		if e.Name() == ".git" || ignored[e.Name()] {
			continue
		}
		full := filepath.Join(resolved, e.Name())
		isDir := e.IsDir()
		if !isDir && e.Type()&os.ModeSymlink != 0 {
			if st, err := os.Stat(full); err == nil {
				isDir = st.IsDir()
			}
		}
		entries = append(entries, dirEntry{Name: e.Name(), Path: full, IsDir: isDir})
	}
	sort.SliceStable(entries, func(i, j int) bool {
		if entries[i].IsDir != entries[j].IsDir {
			return entries[i].IsDir // directories first
		}
		return entries[i].Name < entries[j].Name
	})
	return listEntriesResult{Path: resolved, Entries: entries}, nil
}

// gitIgnored asks git which of `names` are ignored in `dir`. Empty when `dir`
// isn't a repo or git is missing (exit 128) — i.e. no filtering, like the app.
//
// The wire protocol has to match fileService.ts listDir's exactly, or the two
// providers of fs.listEntries disagree about which files exist. Both halves are
// load-bearing and both were bugs:
//
//   - `-z` NUL-delimits stdin AND stdout. A filename may legally contain a
//     newline, and a linefeed-delimited protocol splits `a\nb.log` into the two
//     bogus paths `a` and `b.log`, so the echoed match never equals the readdir
//     name and the ignored file is listed. NUL is the one byte a filename
//     cannot contain.
//   - `core.quotePath=false` keeps non-ASCII names unquoted, so `é.log` comes
//     back as itself rather than as `"\303\251.log"`.
func gitIgnored(dir string, names []string) map[string]bool {
	ignored := map[string]bool{}
	if len(names) == 0 {
		return ignored
	}
	cmd := exec.Command("git", "-c", "core.quotePath=false", "check-ignore", "-z", "--stdin")
	cmd.Dir = dir
	cmd.Stdin = strings.NewReader(strings.Join(names, "\x00"))
	out, err := cmd.Output()
	if err != nil {
		// exit 1 = nothing ignored (stdout still valid); anything else (128 = not
		// a repo / git missing) → no filtering.
		if ee, ok := err.(*exec.ExitError); !ok || ee.ExitCode() != 1 {
			return ignored
		}
	}
	for _, name := range strings.Split(string(out), "\x00") {
		if name != "" {
			ignored[name] = true
		}
	}
	return ignored
}

// supervisorHome ports ensureSupervisorHome: the fleet supervisor's working dir,
// ~/.workspacer, created if missing.
func supervisorHome() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	dir := filepath.Join(home, ".workspacer")
	_ = os.MkdirAll(dir, 0o755)
	return dir
}
