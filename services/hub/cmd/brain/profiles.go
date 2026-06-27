package main

// Claude profiles + argv construction — the part of the "brain" that turns a
// high-level spawn intent (cwd + profile + model) into the argv/env claudemon
// executes. This is a Go port of the logic currently duplicated in the Electron
// app (claudeResolver.ts `buildClaudeArgv`) and the TUI (apps/tui/src/profiles.rs).
// Profiles are read from the same file the app writes: ~/.config/workspacer/claude-profiles.json.

import (
	"crypto/rand"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// profile mirrors a claude-profiles.json entry. configDir becomes
// CLAUDE_CONFIG_DIR; extraArgs is where --model / skip-permissions may be pinned.
type profile struct {
	ID         string   `json:"id"`
	Name       string   `json:"name"`
	ConfigDir  string   `json:"configDir"`
	ExtraArgs  []string `json:"extraArgs"`
	MCPItemIDs []string `json:"mcpItemIds,omitempty"`
	IsDefault  bool     `json:"isDefault"`
}

type profilesFile struct {
	Profiles []profile `json:"profiles"`
}

// configDir is ~/.config/workspacer — the shared config dir the app, the TUI,
// and Claude profiles all use. We honour XDG_CONFIG_HOME like the app does.
func configDir() string {
	if x := os.Getenv("XDG_CONFIG_HOME"); x != "" {
		return filepath.Join(x, "workspacer")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".config", "workspacer")
}

func profilesPath() string {
	return filepath.Join(configDir(), "claude-profiles.json")
}

// loadProfiles reads the configured profiles, always returning at least a
// synthetic "Default" so spawns never fail for lack of a profile (mirrors the
// TUI's load()). The default is ordered first.
func loadProfiles() []profile {
	out := readProfilesFile()
	hasDefault := false
	for _, p := range out {
		if p.IsDefault {
			hasDefault = true
			break
		}
	}
	if !hasDefault {
		out = append([]profile{{ID: "default", Name: "Default", IsDefault: true}}, out...)
	}
	return out
}

func readProfilesFile() []profile {
	text, err := os.ReadFile(profilesPath())
	if err != nil {
		return nil
	}
	var parsed profilesFile
	if err := json.Unmarshal(text, &parsed); err != nil {
		return nil
	}
	return parsed.Profiles
}

// saveProfiles writes the profiles file in the same shape the app does:
// { "profiles": [...] } with 2-space indent.
func saveProfiles(ps []profile) error {
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(map[string][]profile{"profiles": ps}, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(profilesPath(), data, 0o644)
}

// addProfile appends a new profile and persists it, mirroring
// claudeProfiles.addProfile: a fresh uuid id, trimmed configDir, and isDefault
// only when it's the first profile on disk.
func addProfile(name, configDirVal string, extraArgs, mcpItemIDs []string) (*profile, error) {
	if name == "" {
		return nil, fmt.Errorf("claude.profiles.add requires { name }")
	}
	ps := readProfilesFile() // the raw file (no synthetic default), matching the app
	id, err := newSessionID()
	if err != nil {
		return nil, err
	}
	if extraArgs == nil {
		extraArgs = []string{}
	}
	p := profile{
		ID:         id,
		Name:       name,
		ConfigDir:  strings.TrimSpace(configDirVal),
		ExtraArgs:  extraArgs,
		MCPItemIDs: mcpItemIDs,
		IsDefault:  len(ps) == 0,
	}
	ps = append(ps, p)
	if err := saveProfiles(ps); err != nil {
		return nil, err
	}
	return &p, nil
}

// profileUpdate carries the mutable fields of a profile (id is immutable).
type profileUpdate struct {
	Name       *string  `json:"name"`
	ConfigDir  *string  `json:"configDir"`
	ExtraArgs  []string `json:"extraArgs"`
	MCPItemIDs []string `json:"mcpItemIds"`
	IsDefault  *bool    `json:"isDefault"`
}

func updateProfile(id string, u profileUpdate) (*profile, error) {
	ps := readProfilesFile()
	idx := -1
	for i := range ps {
		if ps[i].ID == id {
			idx = i
			break
		}
	}
	if idx < 0 {
		return nil, fmt.Errorf("profile %q not found", id)
	}
	if u.Name != nil {
		ps[idx].Name = *u.Name
	}
	if u.ConfigDir != nil {
		ps[idx].ConfigDir = strings.TrimSpace(*u.ConfigDir)
	}
	if u.ExtraArgs != nil {
		ps[idx].ExtraArgs = u.ExtraArgs
	}
	if u.MCPItemIDs != nil {
		ps[idx].MCPItemIDs = u.MCPItemIDs
	}
	if u.IsDefault != nil && *u.IsDefault {
		for i := range ps {
			ps[i].IsDefault = ps[i].ID == id
		}
	}
	if err := saveProfiles(ps); err != nil {
		return nil, err
	}
	return &ps[idx], nil
}

// removeProfile deletes a profile, refusing to remove the synthetic "default"
// and keeping at least one default, mirroring claudeProfiles.removeProfile.
func removeProfile(id string) error {
	if id == "default" {
		return nil
	}
	ps := readProfilesFile()
	out := ps[:0]
	for _, p := range ps {
		if p.ID != id {
			out = append(out, p)
		}
	}
	hasDefault := false
	for _, p := range out {
		if p.IsDefault {
			hasDefault = true
			break
		}
	}
	if !hasDefault && len(out) > 0 {
		out[0].IsDefault = true
	}
	return saveProfiles(out)
}

func getProfile(id string) *profile {
	if id == "" {
		return nil
	}
	for _, p := range loadProfiles() {
		if p.ID == id {
			pp := p
			return &pp
		}
	}
	return nil
}

func pinsFlag(extraArgs []string, flag string) bool {
	for _, a := range extraArgs {
		if a == flag || strings.HasPrefix(a, flag+"=") {
			return true
		}
	}
	return false
}

// buildArgv builds the argv claudemon should execute for a fresh Claude session,
// mirroring the app's buildClaudeArgv and the TUI's build_argv: base binary,
// then the profile's extra args, then --model / skip-permissions unless the
// profile already pins them. session_id pins --session-id <uuid> so claude names
// its transcript <uuid>.jsonl (the same id we hand claudemon). When resume is
// set, the same id is passed as --resume <uuid> instead; the two are mutually
// exclusive so resume wins. Pass "" for sessionID to skip both (non-claude spawns).
func buildArgv(p *profile, model string, skipPermissions bool, sessionID string, resume bool) []string {
	claude := os.Getenv("WKS_CLAUDE_BIN")
	if claude == "" {
		claude = "claude"
	}
	argv := []string{claude}
	var extra []string
	if p != nil {
		extra = p.ExtraArgs
	}
	argv = append(argv, extra...)

	model = strings.TrimSpace(model)
	if model != "" && !pinsFlag(extra, "--model") {
		argv = append(argv, "--model", model)
	}

	if skipPermissions && !pinsFlag(extra, "--dangerously-skip-permissions") {
		argv = append(argv, "--dangerously-skip-permissions")
	}

	if resume {
		if sessionID != "" {
			argv = append(argv, "--resume", sessionID)
		}
	} else if sessionID != "" && !pinsFlag(extra, "--session-id") {
		argv = append(argv, "--session-id", sessionID)
	}
	return argv
}

// buildEnv returns the env overrides a profile implies — currently just
// CLAUDE_CONFIG_DIR, with a leading ~ expanded.
func buildEnv(p *profile) map[string]string {
	env := map[string]string{}
	if p != nil && p.ConfigDir != "" {
		env["CLAUDE_CONFIG_DIR"] = expandTilde(p.ConfigDir)
	}
	return env
}

func expandTilde(p string) string {
	if strings.HasPrefix(p, "~") {
		if home, err := os.UserHomeDir(); err == nil {
			return home + strings.TrimPrefix(p, "~")
		}
	}
	return p
}

// normalizeCwd tilde-expands and strips trailing slashes. The strip matters:
// claudemon aliases a spawn to Claude's session by exact cwd match, and Claude
// reports its cwd without a trailing slash (mirrors the TUI's normalize_cwd).
func normalizeCwd(p string) string {
	s := expandTilde(strings.TrimSpace(p))
	for len(s) > 1 && strings.HasSuffix(s, "/") {
		s = strings.TrimSuffix(s, "/")
	}
	return s
}

// newSessionID returns a random v4 UUID, used to pin --session-id so our id,
// claude's id, and the transcript filename all agree.
func newSessionID() (string, error) {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		return "", err
	}
	b[6] = (b[6] & 0x0f) | 0x40 // version 4
	b[8] = (b[8] & 0x3f) | 0x80 // variant 10
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16]), nil
}
