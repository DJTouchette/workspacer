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
	"runtime"
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

// configDir is the shared config dir the app, the TUI, and Claude profiles all
// use: %APPDATA%\workspacer on Windows, else $XDG_CONFIG_HOME/workspacer or
// ~/.config/workspacer. This must stay in lockstep with authtoken.ConfigDir()
// and the desktop's getConfigDir — otherwise a headless `workspacer serve`
// reads config/profiles from a different directory than the token store and app.
func configDir() string {
	return configDirFor(runtime.GOOS)
}

// configDirFor resolves the config dir for a given GOOS. Parameterized so the
// Windows branch is unit-testable on any host.
func configDirFor(goos string) string {
	if goos == "windows" {
		if appData := os.Getenv("APPDATA"); appData != "" {
			return filepath.Join(appData, "workspacer")
		}
		home, _ := os.UserHomeDir()
		return filepath.Join(home, "AppData", "Roaming", "workspacer")
	}
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
	// Never serve a nil extraArgs: it marshals as JSON null, and clients (the
	// desktop renderer over the web bridge) index/measure it as an array.
	for i := range out {
		if out[i].ExtraArgs == nil {
			out[i].ExtraArgs = []string{}
		}
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
	return writeFileAtomic(profilesPath(), data, 0o644)
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

// scrubBypassProfile returns the copy of a profile a remote (bus/web/MCP) spawn
// is allowed to use: extraArgs reduced to the allowlist below, and no
// CLAUDE_CONFIG_DIR. Without it, clamping the request's own fields left
// profileId as an open door — the caller points at (or mints, since
// claude.profiles.add is itself a bus capability) a profile that carries the
// bypass for them.
func scrubBypassProfile(p *profile) *profile {
	if p == nil {
		return nil
	}
	cp := *p
	cp.ExtraArgs = scrubBypassArgs(p.ExtraArgs)
	// configDir is dropped rather than contained: it becomes CLAUDE_CONFIG_DIR,
	// and that directory supplies claude's settings.json — permissions.allow and
	// hooks, i.e. commands claude runs unprompted. A bus caller can write files
	// anywhere inside an agent cwd (fs.write) and then name that directory in a
	// profile, so there is no subtree we could allow that the same caller can't
	// also fill in. A remote spawn therefore runs against the host's default
	// claude config dir.
	cp.ConfigDir = ""
	return &cp
}

// remoteSafeFlags is the ALLOWLIST of profile extraArgs that survive onto a
// remote spawn's argv, mapped to whether the flag takes a value. A denylist was
// the wrong shape: it named --dangerously-skip-permissions and a bypass
// --permission-mode, while --allowedTools (blanket tool auto-approval) and
// --settings (an arbitrary settings file: permissions AND hooks) walked straight
// through and handed the bypass back. These three are what a profile legitimately
// pins for a remote spawn; anything else is dropped rather than reasoned about,
// so a flag added to the CLI tomorrow is denied by default.
var remoteSafeFlags = map[string]bool{
	"--model":           true,
	"--effort":          true,
	"--permission-mode": true, // non-bypass modes only — see below
}

// scrubBypassArgs keeps only remoteSafeFlags (both `--flag value` and
// `--flag=value` forms), and drops --permission-mode when it names a bypass mode.
func scrubBypassArgs(args []string) []string {
	out := make([]string, 0, len(args))
	for i := 0; i < len(args); i++ {
		name, value, inline := strings.Cut(args[i], "=")
		takesValue, allowed := remoteSafeFlags[name]
		hasSeparateValue := allowed && takesValue && !inline
		if hasSeparateValue {
			// A flag whose value is the next element, unless it's missing or is
			// itself a flag — in which case the profile is malformed and we drop it.
			if i+1 >= len(args) || strings.HasPrefix(args[i+1], "-") {
				continue
			}
			value = args[i+1]
		}
		if !allowed {
			// Drop the flag AND the value riding beside it: a dropped "--settings"
			// that left its path behind would hand claude a stray positional, which
			// it reads as the prompt.
			if !inline && i+1 < len(args) && !strings.HasPrefix(args[i+1], "-") {
				i++
			}
			continue
		}
		if name == "--permission-mode" && (value == "bypassPermissions" || value == "yolo") {
			if hasSeparateValue {
				i++
			}
			continue
		}
		out = append(out, args[i])
		if hasSeparateValue {
			i++
			out = append(out, value)
		}
	}
	return out
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
// profile already pins them. permissionMode maps to --permission-mode for the
// non-default modes ('bypassPermissions' rides the skip flag instead, and a
// profile that pins a mode wins — same rules as buildClaudeArgv). session_id
// pins --session-id <uuid> so claude names its transcript <uuid>.jsonl (the same
// id we hand claudemon). When resume is set, the same id is passed as
// --resume <uuid> instead; the two are mutually exclusive so resume wins. Pass
// "" for sessionID to skip both (non-claude spawns).
func buildArgv(p *profile, model string, effort string, skipPermissions bool, permissionMode string, sessionID string, resume bool) []string {
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

	// Reasoning-effort maps to --effort, mirroring the desktop's buildClaudeArgv
	// and the brain's own claude-stream path (which forwards p.Effort). A
	// profile that pins --effort wins.
	effort = strings.TrimSpace(effort)
	if effort != "" && !pinsFlag(extra, "--effort") {
		argv = append(argv, "--effort", effort)
	}

	wantsBypass := skipPermissions || permissionMode == "bypassPermissions"
	if wantsBypass && !pinsFlag(extra, "--dangerously-skip-permissions") {
		argv = append(argv, "--dangerously-skip-permissions")
	}

	if permissionMode != "" && permissionMode != "bypassPermissions" && permissionMode != "default" &&
		!wantsBypass && !pinsFlag(extra, "--permission-mode") {
		argv = append(argv, "--permission-mode", permissionMode)
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
