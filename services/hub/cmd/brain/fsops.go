package main

// Host filesystem capabilities — the headless equivalent of the app's fs.* /
// app.getCwd handlers. A web or TUI client browses the host to choose a working
// directory for a new agent, and reads/writes host files for an editor pane.
// These are pure Go (no claudemon, no Electron), so they work identically with
// or without the GUI.

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// listDirResult mirrors the app's fs.listDir return shape: directories only
// (you spawn an agent *in* a folder), hidden entries skipped.
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
	return listDirResult{
		Path:   resolved,
		Parent: filepath.Dir(resolved),
		Home:   home,
		Dirs:   dirs,
	}, nil
}

func readHostFile(p string) (string, error) {
	b, err := os.ReadFile(expandTilde(p))
	if err != nil {
		return "", err
	}
	return string(b), nil
}

func writeHostFile(p, contents string) error {
	return os.WriteFile(expandTilde(p), []byte(contents), 0o644)
}
