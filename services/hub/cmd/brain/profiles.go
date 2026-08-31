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
	"os/user"
	"path/filepath"
	"runtime"
	"strings"
)

// profile mirrors a claude-profiles.json entry. configDir becomes
// CLAUDE_CONFIG_DIR; extraArgs is where --model / skip-permissions may be pinned.
type profile struct {
	ID        string   `json:"id"`
	Name      string   `json:"name"`
	ConfigDir string   `json:"configDir"`
	ExtraArgs []string `json:"extraArgs"`
	// mcpItemIds carries the Library MCP servers a spawn pre-selects. NOT
	// omitempty: main's claude.profiles.add stores `mcpItemIds ?? []`, so the
	// desktop copy always emits the key, and the brain is the DEFAULT answerer
	// for this method (main's `cat()` is a no-op unless WORKSPACER_NO_BRAIN=1).
	// omitempty made the brain's reply and its on-disk record drop the field
	// entirely for a profile with no servers — the same method answering with
	// two different shapes depending on which provider ran. normalizeProfiles
	// keeps it an array rather than the JSON null a nil slice would marshal to.
	MCPItemIDs []string `json:"mcpItemIds"`
	IsDefault  bool     `json:"isDefault"`
	// Weight opts a profile into the desktop's automatic account failover
	// (0 = manual-only). The brain doesn't act on it — but it MUST model it:
	// this file is round-tripped on every add/update/remove, and a field the
	// struct doesn't carry gets wiped from disk by the next brain write.
	// No omitempty, mirroring mcpItemIds: both providers answer one shape.
	// TWIN: `weight` in apps/desktop .../claudeProfiles.ts normalizeProfile.
	Weight int `json:"weight"`
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
//
// Returns "" when there is no home directory to anchor on — see homeDir. Every
// security gate that consumes this already treats a non-absolute config dir as
// unverifiable and fails closed (canonicalRoot discards it, so
// pathIsSecretCanonical answers "secret" for everything), which a RELATIVE
// answer would not have done.
func configDirFor(goos string) string {
	if goos == "windows" {
		if appData := os.Getenv("APPDATA"); appData != "" {
			return filepath.Join(appData, "workspacer")
		}
		home := homeDir()
		if home == "" {
			return ""
		}
		return filepath.Join(home, "AppData", "Roaming", "workspacer")
	}
	if x := os.Getenv("XDG_CONFIG_HOME"); x != "" {
		return filepath.Join(x, "workspacer")
	}
	home := homeDir()
	if home == "" {
		return ""
	}
	return filepath.Join(home, ".config", "workspacer")
}

// homeDir is os.UserHomeDir with the fallback Node's os.homedir() has and Go's
// does not: the effective uid's passwd entry.
//
// os.UserHomeDir reads $HOME and NOTHING ELSE, and it returns an error rather
// than a path when the variable is unset — which a systemd unit, a launchd job
// and most container entrypoints all are. The error was discarded here, so
// filepath.Join("", ".config", "workspacer") produced the RELATIVE string
// ".config/workspacer", resolved against whatever cwd the daemon happened to
// have. Node's os.homedir() falls back to the passwd entry and still answers
// /home/<user>, so the desktop and the TUI went on using the real config dir
// while a headless `workspacer serve` quietly used a different one: config,
// profiles, layouts, sessions and the token store all split in two, and
// downstream in fsguard the relative root was DISCARDED, which turns the
// config-dir half of the secret gate off for the whole process.
//
// "" is the refusal, and it is deliberate: an empty config dir is unverifiable
// everywhere it is consumed and therefore fail-closed, where a relative one
// silently names a real directory.
//
// TWIN: authtoken.HomeDir, and the desktop's os.homedir() (Node builtin).
func homeDir() string {
	if h, err := os.UserHomeDir(); err == nil && h != "" {
		return h
	}
	if u, err := user.Current(); err == nil && u.HomeDir != "" {
		return u.HomeDir
	}
	return ""
}

func profilesPath() string {
	return filepath.Join(configDir(), "claude-profiles.json")
}

// loadProfiles reads the configured profiles, MATERIALIZING the "Default"
// profile when the file has none — the same thing claudeProfiles.ts's
// constructor does, and the reason this used to be a divergence.
//
// It used to PREPEND a synthetic {id:"default"} whenever no profile carried
// isDefault, without ever writing it. Three consequences, all live on
// claude.profiles.*, which the brain answers by default:
//
//   - list returned TWO profiles where the desktop returned one, for the same
//     file. The extra row is not on disk and is not the desktop's "Default".
//   - the brain listed an id its own update REFUSED: updateProfile reads the
//     raw file, so claude.profiles.update("default", ...) answered
//     `profile "default" not found` for a profile claude.profiles.list had just
//     handed the caller.
//   - claude.profiles.add on a fresh config dir minted isDefault:TRUE here and
//     isDefault:FALSE on the desktop, because the desktop's constructor had
//     already persisted the real "default" row and the brain's synthetic one
//     was never on disk for len(ps)==0 to see. That method carries `configDir`,
//     which becomes CLAUDE_CONFIG_DIR.
//
// Materializing makes the listed set, the on-disk set and the set update/remove
// operate on ONE set, on both providers. Pinned by
// contracts/claude-profiles-cases.json.
func loadProfiles() []profile {
	out := readProfilesFile()
	normalizeProfiles(out)
	if len(out) == 0 {
		out = []profile{defaultProfile()}
		// Best effort: a read-only config dir must still yield a usable list.
		_ = saveProfiles(out)
	}
	return out
}

// defaultProfile is the row claudeProfiles.ts's constructor writes when the
// file is empty. Both copies mint exactly this.
func defaultProfile() profile {
	return profile{
		ID:         "default",
		Name:       "Default",
		ConfigDir:  "",
		ExtraArgs:  []string{},
		MCPItemIDs: []string{},
		IsDefault:  true,
	}
}

// normalizeProfiles replaces nil list fields with empty arrays, in place.
// Never serve a nil extraArgs/mcpItemIds: it marshals as JSON null, and clients
// (the desktop renderer over the web bridge) index/measure them as arrays.
// Applied on both read (loadProfiles) and write (saveProfiles) so neither the
// bus reply nor the file on disk can carry a null where main writes [].
func normalizeProfiles(ps []profile) {
	for i := range ps {
		if ps[i].ExtraArgs == nil {
			ps[i].ExtraArgs = []string{}
		}
		if ps[i].MCPItemIDs == nil {
			ps[i].MCPItemIDs = []string{}
		}
	}
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
	normalizeProfiles(ps)
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
	// loadProfiles, not readProfilesFile: the set a caller can see is the set a
	// caller can extend, and it is the same set claudeProfiles.ts's constructor
	// has already materialized on the desktop side.
	ps := loadProfiles()
	id, err := newSessionID()
	if err != nil {
		return nil, err
	}
	if extraArgs == nil {
		extraArgs = []string{}
	}
	// Same default main applies (`mcpItemIds ?? []`): the returned profile is
	// what the caller renders, and it must not differ by provider.
	if mcpItemIDs == nil {
		mcpItemIDs = []string{}
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
	Weight     *int     `json:"weight"`
}

func updateProfile(id string, u profileUpdate) (*profile, error) {
	ps := loadProfiles() // every LISTED id must be updatable
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
	if u.Weight != nil {
		w := *u.Weight
		if w < 0 {
			w = 0
		}
		ps[idx].Weight = w
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
	ps := loadProfiles()
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

// remoteSpawnProfile resolves the profile a bus spawn runs under. Ungranted —
// the doctrine, unchanged since the day profileId was an open door — is
// scrubBypassProfile: bypass extraArgs dropped, configDir dropped, mcpItemIds
// dropped. Granted (profileGranted, which ONLY the hub router stamps, after
// verifying the caller's token record lists this exact profile id —
// internal/bus sanitizeSpawnParams deletes the key from every incoming call
// first, so no caller can be its source) keeps the LOCAL profile's configDir:
// the grant's entire meaning is "dispatch under this account", and the account
// IS the CLAUDE_CONFIG_DIR. That is safe where a wire configDir never was,
// because the id resolves against THIS host's profile store and a profile
// created or updated OVER THE BUS has its configDir scrubbed at write time
// (profilesAdd/profilesUpdate) — so a configDir honored here was typed in by
// the local user, then granted to this token by the local user. The bypass
// scrub still applies to a granted spawn: extraArgs stay allowlisted and
// mcpItemIds stay dropped, because account identity and approval bypass are
// different escalations and the grant only speaks for the first.
func remoteSpawnProfile(profileID string, granted bool) *profile {
	prof := getProfile(profileID)
	if prof == nil {
		return nil
	}
	if !granted {
		return scrubBypassProfile(prof)
	}
	cp := scrubBypassProfile(prof)
	cp.ConfigDir = prof.ConfigDir
	return cp
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
	// mcpItemIds goes with it, for the same reason and with a sharper edge. A
	// library item of kind `mcp` carries `command`, `args` and `env` verbatim
	// into a --mcp-config file, and the spawn passes `--allowedTools mcp__<id>`
	// alongside it, so the server is PRE-APPROVED and no prompt gates it: a
	// persisted id list is a persisted argv[0]. It used to be forwarded PAST this
	// scrub on both providers — the one field capspec's "scrubbed at write time
	// on both bus providers" record did not actually cover — and the desktop's
	// New Agent dialog copies a profile's mcpItemIds into the spawn the moment
	// the profile is selected, so a bus-planted profile loaded the caller's MCP
	// servers into a LOCAL spawn. Nil, not empty: normalizeProfiles turns it back
	// into [] for the wire.
	cp.MCPItemIDs = nil
	return &cp
}

// remoteSafeFlags is the ALLOWLIST of profile extraArgs that survive onto a
// remote spawn's argv, mapped to whether the flag takes a value. A denylist was
// the wrong shape: it named --dangerously-skip-permissions and a bypass
// --permission-mode, while --allowedTools (blanket tool auto-approval) and
// --settings (an arbitrary settings file: permissions AND hooks) walked straight
// through and handed the bypass back. These four are what a profile legitimately
// pins for a remote spawn; anything else is dropped rather than reasoned about,
// so a flag added to the CLI tomorrow is denied by default.
var remoteSafeFlags = map[string]bool{
	"--model":                true,
	"--effort":               true,
	"--permission-mode":      true, // non-bypass modes only — see below
	"--append-system-prompt": true,
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

// modelFromArgs returns the last executable profile model. buildArgv lets a
// profile pin win over the ordinary/default model, so the canonical pair sent
// to claudemon must be derived from this same value.
func modelFromArgs(extraArgs []string) string {
	model := ""
	for i := 0; i < len(extraArgs); i++ {
		arg := extraArgs[i]
		switch {
		case arg == "--model" && i+1 < len(extraArgs) && !strings.HasPrefix(extraArgs[i+1], "--"):
			if value := strings.TrimSpace(extraArgs[i+1]); value != "" {
				model = value
			}
		case strings.HasPrefix(arg, "--model="):
			if value := strings.TrimSpace(strings.TrimPrefix(arg, "--model=")); value != "" {
				model = value
			}
		}
	}
	return model
}

// composeAppendSystemPrompt gives Claude exactly one append-system-prompt flag.
// Profile pins and host contracts are additive, but Claude does not define
// repeated flags as concatenation. Preserve the profile's declaration order,
// then host-authored fragments in the argv order that follows it, with the same
// blank-line separator as the desktop's partitionAppendSystemPrompts.
func composeAppendSystemPrompt(argv []string) []string {
	withoutPrompts := make([]string, 0, len(argv))
	prompts := make([]string, 0, 2)
	for i := 0; i < len(argv); i++ {
		arg := argv[i]
		if strings.HasPrefix(arg, "--append-system-prompt=") {
			if value := strings.TrimPrefix(arg, "--append-system-prompt="); value != "" {
				prompts = append(prompts, value)
			}
			continue
		}
		if arg == "--append-system-prompt" {
			if i+1 < len(argv) && !strings.HasPrefix(argv[i+1], "--") {
				prompts = append(prompts, argv[i+1])
				i++
			}
			// A valueless profile pin is invalid on its own. Drop only that
			// broken flag rather than letting it consume a host-generated one.
			continue
		}
		withoutPrompts = append(withoutPrompts, arg)
	}
	if len(prompts) > 0 {
		withoutPrompts = append(withoutPrompts, "--append-system-prompt", strings.Join(prompts, "\n\n"))
	}
	return withoutPrompts
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

// asciiWhitespace is the whitespace every caller-string trim on this seam
// strips: space, tab, and the four ASCII vertical/form controls. Spelled as a
// literal set because neither language's built-in trim is portable — see
// normalizeCwd's TRIM SET note. Twins: asciiWhitespace.ts (isAsciiBlank /
// hasNonBlankText), searchService.ts TEXT_TRIM and spawnCwd.ts TRIM_SET.
const asciiWhitespace = " \t\n\v\f\r"

// normalizeCwd is the ONE normalization a caller-supplied spawn/terminal cwd
// gets, on both providers. It trims surrounding whitespace and strips trailing
// slashes, and does nothing else.
//
// The strip matters: claudemon aliases a spawn to Claude's session by exact cwd
// match, and Claude reports its cwd without a trailing slash (mirrors the TUI's
// normalize_cwd).
//
// It deliberately does NOT expand '~' (BINDING DECISION 1, fsguard.go's header).
// It used to, and the desktop twin never did, so `agents.spawn {"cwd":"~"}` was
// $HOME on this provider and the literal string "~" on the other. That is not
// cosmetic: the stored session cwd is what agentCwds() feeds into
// workspaceRoots(), so one caller string turned the ENTIRE home tree into an
// fs.* root here and into nothing at all there — the same allowed-by-one-
// provider / denied-by-the-other split the tilde rule exists to close.
//
// It also does NOT check existence or fall back to $HOME. The desktop's
// terminals.create used to (`fs.existsSync(cwd) ? cwd : os.homedir()`), which
// silently rewrote the caller's target to somewhere else entirely; a spawn that
// cannot run where it was asked to should fail where it was asked to.
//
// TWIN: apps/desktop/src/main/lib/spawnCwd.ts normalizeSpawnCwd. The fixture's
// `spawnCwds` block holds the two together.
//
// The TRIM SET is spelled out rather than delegated. strings.TrimSpace and JS
// `.trim()` are not the same function, and the two differences point in opposite
// directions:
//
//	U+FEFF (ZWNBSP/BOM)  in ECMAScript's WhiteSpace production, NOT in Go's
//	                     unicode.IsSpace (dropped from White_Space in Unicode
//	                     4.0.1) — so {"cwd":"<U+FEFF>"} trimmed to empty on the
//	                     desktop and became $HOME, while the brain ran a session
//	                     in a directory literally named U+FEFF.
//	U+0085 (NEL)         unicode.IsSpace in Go, neither <USP> nor a
//	                     LineTerminator in JS — the same split, the other way.
//
// A BOM is exactly what a path pasted out of a Windows editor or read from a
// UTF-8-with-BOM file carries, and this cwd is the string that lands in
// workspaceRoots(), so "$HOME is a root" versus "a nonexistent directory is a
// root" is the difference. Neither language's built-in is portable, so both
// copies trim the ASCII whitespace set and nothing else; every other code point
// is an ordinary character in a filename, which is what it is on the filesystem.
func normalizeCwd(p string) string {
	s := strings.Trim(p, asciiWhitespace)
	for len(s) > 1 && (strings.HasSuffix(s, "/") || strings.HasSuffix(s, "\\")) {
		s = s[:len(s)-1]
	}
	if s == "" {
		// A terminal has to open SOMEWHERE. Both call sites did this
		// individually; it lives here so the twins are one function.
		home, _ := os.UserHomeDir()
		return home
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
