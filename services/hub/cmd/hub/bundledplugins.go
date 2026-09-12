package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

func bundledExamplesDir() string {
	if executable, err := os.Executable(); err == nil {
		candidate := filepath.Join(filepath.Dir(executable), "examples")
		if info, err := os.Stat(candidate); err == nil && info.IsDir() {
			return candidate
		}
	}
	candidate := "/usr/local/share/workspacer/examples"
	if info, err := os.Stat(candidate); err == nil && info.IsDir() {
		return candidate
	}
	return ""
}

// Matches the desktop's first-run webview-only defaults. Never replaces an
// existing plugin set and never executes an install script or sidecar.
func seedBundledPlugins(destination, examples string) error {
	if destination == "" || examples == "" {
		return nil
	}
	if err := os.MkdirAll(destination, 0700); err != nil {
		return err
	}
	entries, err := os.ReadDir(destination)
	if err != nil {
		return err
	}
	if len(entries) != 0 {
		return nil
	}
	for _, name := range []string{"editor", "transcript-timeline"} {
		source := filepath.Join(examples, name)
		if _, err := os.Stat(source); os.IsNotExist(err) {
			continue
		}
		temporary, err := os.MkdirTemp(destination, ".seed-")
		if err != nil {
			return err
		}
		staged := filepath.Join(temporary, name)
		err = os.CopyFS(staged, os.DirFS(source))
		if err == nil {
			manifest, loadErr := plugin.Load(filepath.Join(staged, "plugin.json"))
			err = loadErr
			if err == nil && (manifest.ID != "workspacer."+name || manifest.Server != nil || len(manifest.Install) != 0) {
				err = fmt.Errorf("bundled %s must be its expected webview-only plugin", name)
			}
		}
		if err == nil {
			err = os.Rename(staged, filepath.Join(destination, name))
		}
		_ = os.RemoveAll(temporary)
		if err != nil {
			return err
		}
	}
	return nil
}
