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

func listHostDir(p string) (listDirResult, error) {
	home, _ := os.UserHomeDir()
	target := home
	if s := strings.TrimSpace(p); s != "" {
		target = expandTilde(s)
	}
	resolved, err := filepath.Abs(target)
	if err != nil {
		return listDirResult{}, err
	}
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
func readTextFile(p string) (*readFileResult, error) {
	full := expandTilde(p)
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

func writeHostFile(p, contents string) error {
	return os.WriteFile(expandTilde(p), []byte(contents), 0o644)
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
func listEntries(dirPath string) (listEntriesResult, error) {
	resolved, err := filepath.Abs(expandTilde(dirPath))
	if err != nil {
		return listEntriesResult{}, err
	}
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
func gitIgnored(dir string, names []string) map[string]bool {
	ignored := map[string]bool{}
	if len(names) == 0 {
		return ignored
	}
	cmd := exec.Command("git", "-c", "core.quotePath=false", "check-ignore", "--stdin")
	cmd.Dir = dir
	cmd.Stdin = strings.NewReader(strings.Join(names, "\n"))
	out, err := cmd.Output()
	if err != nil {
		// exit 1 = nothing ignored (stdout still valid); anything else (128 = not
		// a repo / git missing) → no filtering.
		if ee, ok := err.(*exec.ExitError); !ok || ee.ExitCode() != 1 {
			return ignored
		}
	}
	for _, line := range strings.Split(string(out), "\n") {
		if line != "" {
			ignored[line] = true
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
