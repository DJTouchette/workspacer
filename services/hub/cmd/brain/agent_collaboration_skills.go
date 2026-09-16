package main

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// installHeadlessAgentCollaborationSkills mirrors the desktop's immutable,
// pointer-only installer. The generated map and version hash come from the
// exact same assets; prompts carry paths, never a copy of every SKILL.md body.
func installHeadlessAgentCollaborationSkills(provider, cwd string, manager bool) string {
	root, ok := safeHeadlessSkillRoot(cwd)
	if !ok {
		return ""
	}
	removeLegacyHeadlessSkillCopies(provider, cwd)
	if manager || provider == "pi" {
		return ""
	}
	if !installHeadlessSkillFiles(root, cwd) {
		return ""
	}
	return fmt.Sprintf(
		"Workspacer provides two project skills: read %q before spawning child agents, and %q before maintaining the project brief.",
		filepath.Join(root, "spawn-agent", "SKILL.md"),
		filepath.Join(root, "project-brief", "SKILL.md"),
	)
}

func safeHeadlessSkillRoot(cwd string) (string, bool) {
	if strings.TrimSpace(cwd) == "" {
		return "", false
	}
	original, err := filepath.Abs(cwd)
	if err != nil {
		return "", false
	}
	info, err := os.Lstat(original)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return "", false
	}
	real, err := filepath.EvalSymlinks(original)
	if err != nil {
		return "", false
	}
	home, _ := os.UserHomeDir()
	if home != "" {
		home, _ = filepath.EvalSymlinks(home)
	}
	if real == filepath.VolumeName(real)+string(filepath.Separator) || (home != "" && real == home) {
		return "", false
	}
	return filepath.Join(real, ".workspacer", "skills", headlessAgentCollaborationSkillsVersion), true
}

func installHeadlessSkillFiles(root, cwd string) bool {
	real, err := filepath.EvalSymlinks(cwd)
	if err != nil {
		return false
	}
	rel, err := filepath.Rel(real, root)
	if err != nil || rel == "." || filepath.IsAbs(rel) || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return false
	}
	current := real
	for _, part := range strings.Split(rel, string(filepath.Separator)) {
		current = filepath.Join(current, part)
		if err := os.Mkdir(current, 0o755); err != nil && !errors.Is(err, os.ErrExist) {
			return false
		}
		info, err := os.Lstat(current)
		if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return false
		}
	}

	keys := make([]string, 0, len(headlessAgentCollaborationSkillFiles))
	for rel := range headlessAgentCollaborationSkillFiles {
		keys = append(keys, rel)
	}
	sort.Strings(keys)
	// Preflight every collision before writing any file.
	for _, rel := range keys {
		file := filepath.Join(root, filepath.FromSlash(rel))
		if !strings.HasPrefix(file, root+string(filepath.Separator)) {
			return false
		}
		info, err := os.Lstat(file)
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 {
			return false
		}
		body, err := os.ReadFile(file)
		if err != nil || string(body) != headlessAgentCollaborationSkillFiles[rel] {
			return false
		}
	}
	for _, rel := range keys {
		file := filepath.Join(root, filepath.FromSlash(rel))
		parent := filepath.Dir(file)
		if err := os.Mkdir(parent, 0o755); err != nil && !errors.Is(err, os.ErrExist) {
			return false
		}
		if info, err := os.Lstat(parent); err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return false
		}
		f, err := os.OpenFile(file, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
		if errors.Is(err, os.ErrExist) {
			body, readErr := os.ReadFile(file)
			if readErr != nil || string(body) != headlessAgentCollaborationSkillFiles[rel] {
				return false
			}
			continue
		}
		if err != nil {
			return false
		}
		_, writeErr := f.WriteString(headlessAgentCollaborationSkillFiles[rel])
		closeErr := f.Close()
		if writeErr != nil || closeErr != nil {
			return false
		}
	}
	return true
}

func removeLegacyHeadlessSkillCopies(provider, cwd string) {
	native := ""
	switch provider {
	case "", "claude":
		native = ".claude"
	case "codex":
		native = ".agents"
	default:
		return
	}
	for rel, want := range headlessAgentCollaborationSkillFiles {
		file := filepath.Join(cwd, native, "skills", filepath.FromSlash(rel))
		info, err := os.Lstat(file)
		if err != nil || !info.Mode().IsRegular() || info.Mode()&os.ModeSymlink != 0 {
			continue
		}
		body, err := os.ReadFile(file)
		if err != nil || string(body) != want {
			continue
		}
		_ = os.Remove(file)
		_ = os.Remove(filepath.Dir(file))
	}
}
