package main

import (
	"context"
	"encoding/json"
	"os"
	"os/user"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

// argv tests ported from apps/tui/src/profiles.rs so the Go brain stays in
// lockstep with the TUI/app argv logic.

func hasPair(argv []string, a, b string) bool {
	for i := 0; i+1 < len(argv); i++ {
		if argv[i] == a && argv[i+1] == b {
			return true
		}
	}
	return false
}

func TestResumeUsesResumeFlagNotSessionID(t *testing.T) {
	argv := buildArgv(&profile{}, "", "", false, "", "abc-123", true)
	if !hasPair(argv, "--resume", "abc-123") {
		t.Fatalf("expected --resume abc-123, got %v", argv)
	}
	if slices.Contains(argv, "--session-id") {
		t.Fatalf("resume must not add --session-id, got %v", argv)
	}
}

func TestFreshSpawnUsesSessionIDNotResume(t *testing.T) {
	argv := buildArgv(&profile{}, "", "", false, "", "abc-123", false)
	if !hasPair(argv, "--session-id", "abc-123") {
		t.Fatalf("expected --session-id abc-123, got %v", argv)
	}
	if slices.Contains(argv, "--resume") {
		t.Fatalf("fresh spawn must not add --resume, got %v", argv)
	}
}

func TestProfilePinnedModelNotDuplicated(t *testing.T) {
	p := &profile{ExtraArgs: []string{"--model", "opus"}}
	argv := buildArgv(p, "sonnet", "", false, "", "", false)
	n := 0
	for _, a := range argv {
		if a == "--model" {
			n++
		}
	}
	if n != 1 {
		t.Fatalf("expected exactly one --model, got %d in %v", n, argv)
	}
	if slices.Contains(argv, "sonnet") {
		t.Fatalf("requested model must not override a profile-pinned one, got %v", argv)
	}
}

func TestSkipPermissionsAddedOnce(t *testing.T) {
	argv := buildArgv(&profile{}, "", "", true, "", "", false)
	if !slices.Contains(argv, "--dangerously-skip-permissions") {
		t.Fatalf("expected skip-permissions flag, got %v", argv)
	}
	// Not added when the profile already pins it.
	p := &profile{ExtraArgs: []string{"--dangerously-skip-permissions"}}
	argv = buildArgv(p, "", "", true, "", "", false)
	n := 0
	for _, a := range argv {
		if a == "--dangerously-skip-permissions" {
			n++
		}
	}
	if n != 1 {
		t.Fatalf("expected one skip flag, got %d in %v", n, argv)
	}
}

func TestPermissionModeFlag(t *testing.T) {
	// Non-default modes map to --permission-mode (mirrors buildClaudeArgv).
	argv := buildArgv(&profile{}, "", "", false, "plan", "", false)
	if !hasPair(argv, "--permission-mode", "plan") {
		t.Fatalf("expected --permission-mode plan, got %v", argv)
	}
	// 'default' adds no flag.
	argv = buildArgv(&profile{}, "", "", false, "default", "", false)
	if slices.Contains(argv, "--permission-mode") {
		t.Fatalf("'default' must not add --permission-mode, got %v", argv)
	}
	// 'bypassPermissions' rides the skip flag, never --permission-mode.
	argv = buildArgv(&profile{}, "", "", false, "bypassPermissions", "", false)
	if !slices.Contains(argv, "--dangerously-skip-permissions") || slices.Contains(argv, "--permission-mode") {
		t.Fatalf("bypass must map to the skip flag only, got %v", argv)
	}
	// A profile-pinned mode wins over the requested one.
	p := &profile{ExtraArgs: []string{"--permission-mode", "acceptEdits"}}
	argv = buildArgv(p, "", "", false, "plan", "", false)
	n := 0
	for _, a := range argv {
		if a == "--permission-mode" {
			n++
		}
	}
	if n != 1 || slices.Contains(argv, "plan") {
		t.Fatalf("profile-pinned mode must win, got %v", argv)
	}
}

func TestBaseBinaryAndExtraArgsOrder(t *testing.T) {
	p := &profile{ExtraArgs: []string{"--foo", "bar"}}
	argv := buildArgv(p, "", "", false, "", "", false)
	if len(argv) < 3 || argv[0] != "claude" || argv[1] != "--foo" || argv[2] != "bar" {
		t.Fatalf("expected [claude --foo bar ...], got %v", argv)
	}
}

func TestNormalizeCwdStripsTrailingSlashes(t *testing.T) {
	cases := map[string]string{
		"/home/u/backshop/":     "/home/u/backshop",
		"/home/u/backshop///":   "/home/u/backshop",
		"  /home/u/backshop/  ": "/home/u/backshop",
		"/home/u/backshop":      "/home/u/backshop",
		"/":                     "/",
	}
	for in, want := range cases {
		if got := normalizeCwd(in); got != want {
			t.Errorf("normalizeCwd(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestBuildEnvSetsConfigDir(t *testing.T) {
	env := buildEnv(&profile{ConfigDir: "/tmp/cfg"})
	if env["CLAUDE_CONFIG_DIR"] != "/tmp/cfg" {
		t.Fatalf("expected CLAUDE_CONFIG_DIR, got %v", env)
	}
	if env := buildEnv(&profile{}); len(env) != 0 {
		t.Fatalf("expected empty env for no configDir, got %v", env)
	}
}

func TestNewSessionIDLooksLikeUUIDv4(t *testing.T) {
	id, err := newSessionID()
	if err != nil {
		t.Fatal(err)
	}
	parts := strings.Split(id, "-")
	if len(parts) != 5 || len(parts[0]) != 8 || len(parts[2]) != 4 || parts[2][0] != '4' {
		t.Fatalf("not a v4 uuid: %q", id)
	}
}

// TestEffortFlagEmittedAndProfilePinWins covers idx 24: a PTY Claude spawn must
// honor reasoning-effort (--effort), matching buildClaudeArgv and the brain's
// own stream path, with a profile-pinned effort winning over the request.
func TestEffortFlagEmittedAndProfilePinWins(t *testing.T) {
	argv := buildArgv(&profile{}, "", "high", false, "", "", false)
	if !hasPair(argv, "--effort", "high") {
		t.Fatalf("expected --effort high, got %v", argv)
	}

	// Empty effort adds no flag.
	argv = buildArgv(&profile{}, "", "", false, "", "", false)
	if slices.Contains(argv, "--effort") {
		t.Fatalf("empty effort must not add --effort, got %v", argv)
	}

	// A whitespace-only effort is trimmed to empty and adds no flag.
	argv = buildArgv(&profile{}, "", "  ", false, "", "", false)
	if slices.Contains(argv, "--effort") {
		t.Fatalf("blank effort must not add --effort, got %v", argv)
	}

	// A profile-pinned effort wins over the requested one (same rule as --model).
	p := &profile{ExtraArgs: []string{"--effort", "medium"}}
	argv = buildArgv(p, "", "high", false, "", "", false)
	n := 0
	for _, a := range argv {
		if a == "--effort" {
			n++
		}
	}
	if n != 1 || slices.Contains(argv, "high") {
		t.Fatalf("profile-pinned effort must win, got %v", argv)
	}
}

// TestConfigDirWindowsHonorsAppData pins the brain's config dir resolution to
// the same layout as authtoken.ConfigDir / the desktop app: %APPDATA%\workspacer
// on Windows. Covers idx 19.
func TestConfigDirWindowsHonorsAppData(t *testing.T) {
	t.Setenv("APPDATA", `C:\Users\me\AppData\Roaming`)

	got := configDirFor("windows")
	want := filepath.Join(`C:\Users\me\AppData\Roaming`, "workspacer")
	if got != want {
		t.Fatalf("windows configDir = %q, want %q (must match authtoken.ConfigDir / desktop app)", got, want)
	}
}

func TestConfigDirWindowsFallsBackToRoaming(t *testing.T) {
	t.Setenv("APPDATA", "")
	home, _ := os.UserHomeDir()

	got := configDirFor("windows")
	want := filepath.Join(home, "AppData", "Roaming", "workspacer")
	if got != want {
		t.Fatalf("windows configDir fallback = %q, want %q", got, want)
	}
}

// configDirFor's header binds three copies — "this must stay in lockstep with
// authtoken.ConfigDir() and the desktop's getConfigDir" — and with no HOME they
// came apart.
//
// Go's os.UserHomeDir reads $HOME and nothing else; the error was discarded, so
// filepath.Join("", ".config", "workspacer") answered the RELATIVE
// ".config/workspacer", resolved against whatever cwd a systemd unit or a
// container entrypoint happened to have. Node's os.homedir() falls back to the
// effective uid's passwd entry, so the desktop kept using the real directory:
// one headless `workspacer serve` reading its config, profiles, layouts,
// sessions and token store from somewhere else entirely. Downstream in fsguard
// the relative root is DISCARDED, which switches the config-dir half of the
// secret gate off for the whole process.
//
// Two claims, and the second is the one that could regress silently:
//
//  1. ABSOLUTE, always. A relative config dir is never an acceptable answer.
//  2. THE SAME absolute path from both Go copies, which is what "in lockstep"
//     means — a fallback added to one of them is not a fix.
func TestConfigDirResolvesWithoutHOME(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("HOME is not how Windows resolves a home directory")
	}
	t.Setenv("XDG_CONFIG_HOME", "")
	t.Setenv("HOME", "")
	os.Unsetenv("HOME")

	got := configDirFor("linux")

	// Whether this host HAS a passwd home decides the RIGHT answer, and it is
	// computed independently of `got` — because the regression this test exists to
	// catch (dropping the passwd fallback Go's os.UserHomeDir lacks and Node's
	// os.homedir() has) makes `got` go EMPTY, and the old skip guard `if got == ""`
	// could not tell that apart from "this host genuinely has no home". So it
	// self-skipped GREEN on the very break it guards. Only a host with no passwd
	// home may skip; a host that has one and answers "" has regressed.
	u, err := user.Current()
	if err != nil || u.HomeDir == "" {
		t.Skip("this host has no passwd home for the effective uid either — nothing to compare")
	}

	if got == "" {
		t.Fatalf("with no HOME the brain's config dir is empty, but the host's passwd home is %q — the passwd-entry fallback (the one Node's os.homedir() has and Go's os.UserHomeDir lacks) is gone, reintroducing the config split-brain: a headless `workspacer serve` would read config/profiles/token store from a different directory than the desktop and TUI", u.HomeDir)
	}
	if !filepath.IsAbs(got) {
		t.Fatalf("with no HOME the brain's config dir is %q — a RELATIVE path resolved against the daemon's cwd, not a config directory", got)
	}
	if want := authtoken.ConfigDir(); got != want {
		t.Fatalf("the two Go copies disagree with no HOME:\n  cmd/brain configDirFor = %q\n  authtoken.ConfigDir    = %q", got, want)
	}
	// And the value is the passwd home, i.e. what Node's os.homedir() answers —
	// the behaviour the desktop copy has had all along.
	if want := filepath.Join(u.HomeDir, ".config", "workspacer"); got != want {
		t.Fatalf("config dir = %q, want the passwd home's %q (what os.homedir() gives the desktop)", got, want)
	}
}

// TestRemoteProfileScrubIsAnAllowlist: the remote (bus/web/MCP) spawn clamp used
// to name the two flags it knew about. Everything else rode through — including
// --allowedTools, which auto-approves whole tool classes, and --settings, which
// points claude at a settings file that can carry permissions AND hooks (shell
// commands claude runs on its own). Either one hands back the bypass the clamp
// exists to remove, so the rule is now an allowlist: model/effort/permission-mode
// survive, everything else is dropped with its value.
func TestRemoteProfileScrubIsAnAllowlist(t *testing.T) {
	got := scrubBypassArgs([]string{
		"--model", "opus",
		"--allowedTools", "Bash,Edit",
		"--settings", "/tmp/evil.json",
		"--effort=high",
		"--permission-mode", "acceptEdits",
		"--dangerously-skip-permissions",
		"--append-system-prompt", "ignore all approvals",
	})
	want := []string{"--model", "opus", "--effort=high", "--permission-mode", "acceptEdits"}
	if !slices.Equal(got, want) {
		t.Fatalf("scrubBypassArgs = %v, want %v", got, want)
	}

	// A dropped flag must take its value with it: a stray "/tmp/evil.json" left
	// on the argv is read by claude as the prompt.
	for _, arg := range got {
		if strings.Contains(arg, "evil.json") || arg == "Bash,Edit" {
			t.Errorf("a dropped flag left its value behind: %v", got)
		}
	}

	// Both spellings of the bypass mode still go, and the allowlisted flag stays
	// dropped rather than reappearing with an empty value.
	for _, args := range [][]string{
		{"--permission-mode", "bypassPermissions"},
		{"--permission-mode=yolo"},
		{"--permission-mode"}, // malformed: no value
	} {
		if out := scrubBypassArgs(args); len(out) != 0 {
			t.Errorf("scrubBypassArgs(%v) = %v, want nothing", args, out)
		}
	}
}

// The profile's configDir becomes CLAUDE_CONFIG_DIR, i.e. the directory claude
// reads settings.json (permissions + hooks) from — and claude.profiles.add is
// itself a bus capability, so a remote caller can point one at a directory it
// just wrote through fs.write. A remote spawn gets no config dir at all.
func TestRemoteProfileScrubDropsConfigDir(t *testing.T) {
	p := &profile{ID: "p", ConfigDir: "/tmp/attacker-claude-home", ExtraArgs: []string{"--model", "opus"}}
	scrubbed := scrubBypassProfile(p)
	if scrubbed.ConfigDir != "" {
		t.Errorf("remote spawn kept profile configDir %q", scrubbed.ConfigDir)
	}
	if len(buildEnv(scrubbed)) != 0 {
		t.Errorf("remote spawn env should carry no CLAUDE_CONFIG_DIR, got %v", buildEnv(scrubbed))
	}
	if p.ConfigDir == "" {
		t.Error("scrubBypassProfile must not mutate the stored profile")
	}
}

// ── claude.profiles.add: the mcpItemIds twin of apps/desktop/tests/main/
// hubCapabilitiesProfiles.test.ts ──────────────────────────────────────────
//
// That TS file pins two behaviours of main's claude.profiles.add — it forwards
// mcpItemIds, and it defaults them to [] — and it has to force
// DELEGATE_CATALOG_TO_BRAIN=false to reach the handler at all, because under
// the shipping default main's `cat()` registers nothing and THIS provider
// answers the method. The pins below are the same two behaviours on the copy
// that actually runs, driven through reg.handle so the param decoding in
// profilesAdd is covered too, not just addProfile.

// readProfilesJSON returns the raw decoded claude-profiles.json, so a test can
// assert on key presence (absent vs null vs []) rather than Go zero values.
func readProfilesJSON(t *testing.T) []map[string]any {
	t.Helper()
	data, err := os.ReadFile(profilesPath())
	if err != nil {
		t.Fatalf("reading %s: %v", profilesPath(), err)
	}
	var parsed struct {
		Profiles []map[string]any `json:"profiles"`
	}
	if err := json.Unmarshal(data, &parsed); err != nil {
		t.Fatalf("claude-profiles.json is not valid JSON: %v", err)
	}
	return parsed.Profiles
}

// mcpItemIds is SCRUBBED on a bus write, not forwarded. This test used to pin
// the opposite, and the forwarding was deliberate — the comment above the call
// said the web/remote client sends the user's selected MCP servers here.
//
// What that missed: a library item of kind `mcp` carries `command`, `args` and
// `env` verbatim into a --mcp-config file, and the spawn passes
// `--allowedTools mcp__<id>` alongside it, so the server is PRE-APPROVED and no
// permission prompt gates it. A persisted id list is a persisted argv[0]. And
// the id resolves against a library a bus caller can write — through
// library.save, or through a plain fs.write into <configDir>/library, which is a
// configStoreRoot by design — so there is nothing to validate on the way in.
// SpawnAgentDialog copies a profile's mcpItemIds into the spawn the moment the
// profile is selected, which is precisely the "wait for the LOCAL user to pick
// that profile, where nothing scrubs" escalation scrubBypassProfile exists to
// close, through the one field it did not cover.
func TestProfilesAddScrubsMcpItemIdsAtWriteTime(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	reg := newRegistry(newClaudemonClient("http://unused"))

	res, err := reg.handle(context.Background(), "claude.profiles.add",
		[]byte(`{"name":"P","extraArgs":["--model","opus"],"mcpItemIds":["mcp-1","mcp-2"]}`))
	if err != nil {
		t.Fatalf("claude.profiles.add: %v", err)
	}

	var got profile
	if err := json.Unmarshal(res, &got); err != nil {
		t.Fatalf("result not valid JSON: %v", err)
	}
	if len(got.MCPItemIDs) != 0 {
		t.Errorf("a bus write persisted mcpItemIds %v — each id becomes argv[0] of a host process, pre-approved via --allowedTools", got.MCPItemIDs)
	}
	// The rest of the forwarding main pins in the same call, so a param-name
	// typo here can't hide behind the mcpItemIds assertion. extraArgs is spelled
	// with a REMOTE-SAFE flag: every call this brain answers arrives over the
	// bus, and the write is scrubbed — see
	// TestProfilesWritesOverTheBusAreScrubbedAtWriteTime for the dropping half.
	if got.Name != "P" || !slices.Equal(got.ExtraArgs, []string{"--model", "opus"}) {
		t.Errorf("add mangled the other fields: %+v", got)
	}
	if got.ID == "" {
		t.Error("add returned a profile with no id")
	}

	// Scrubbed on DISK too — a spawn reads the file, not the reply — and still
	// present as [], because the desktop twin always emits the key and the two
	// providers must answer with the same shape.
	stored := lastStoredProfile(t)
	if !reflect.DeepEqual(stored["mcpItemIds"], []any{}) {
		t.Errorf("stored profile's mcpItemIds is %v, want an empty array", stored["mcpItemIds"])
	}

	// And through update, the other way to plant one on a profile the local user
	// then picks.
	if _, err := reg.handle(context.Background(), "claude.profiles.update",
		[]byte(`{"id":"`+got.ID+`","updates":{"mcpItemIds":["mcp-3"]}}`)); err != nil {
		t.Fatalf("claude.profiles.update: %v", err)
	}
	if ids := lastStoredProfile(t)["mcpItemIds"]; !reflect.DeepEqual(ids, []any{}) {
		t.Errorf("claude.profiles.update persisted mcpItemIds %v", ids)
	}
}

func TestProfilesAddDefaultsMcpItemIds(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	reg := newRegistry(newClaudemonClient("http://unused"))

	res, err := reg.handle(context.Background(), "claude.profiles.add", []byte(`{"name":"P"}`))
	if err != nil {
		t.Fatalf("claude.profiles.add: %v", err)
	}

	// main returns mcpItemIds: [] here. Decode to a map: a nil Go slice would
	// pass a slices.Equal against []string{} while marshalling to null, and an
	// omitempty tag would drop the key entirely — both are shapes main never
	// emits, and both are invisible to a typed decode.
	var reply map[string]any
	if err := json.Unmarshal(res, &reply); err != nil {
		t.Fatalf("result not valid JSON: %v", err)
	}
	ids, ok := reply["mcpItemIds"]
	if !ok {
		t.Fatalf("reply omits mcpItemIds entirely; main emits []. got %v", reply)
	}
	if !reflect.DeepEqual(ids, []any{}) {
		t.Errorf("mcpItemIds should default to [], got %#v", ids)
	}
	// Same for extraArgs/configDir, the other two main defaults ('' and []).
	if !reflect.DeepEqual(reply["extraArgs"], []any{}) || reply["configDir"] != "" {
		t.Errorf("add should default configDir='' and extraArgs=[], got %#v / %#v",
			reply["configDir"], reply["extraArgs"])
	}

	// Two rows: the materialized "Default" (which claudeProfiles.ts's constructor
	// writes on the desktop side, and which the brain used to only PRETEND was
	// there) plus the one just added.
	stored := lastStoredProfile(t)
	if v, ok := stored["mcpItemIds"]; !ok || !reflect.DeepEqual(v, []any{}) {
		t.Errorf("stored profile should carry mcpItemIds: [], got %#v (present=%v)", v, ok)
	}
}

// A profile written before mcpItemIds existed has no such key. claude.profiles
// .list must still hand clients an array — the renderer does `?? []` today, but
// the brain's own guarantee is the array, and a JSON null is what a nil slice
// marshals to.
func TestProfilesListNeverServesNullLists(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	legacy := `{"profiles":[{"id":"old","name":"Old","configDir":"","isDefault":true}]}`
	if err := os.WriteFile(profilesPath(), []byte(legacy), 0o644); err != nil {
		t.Fatal(err)
	}

	reg := newRegistry(newClaudemonClient("http://unused"))
	res, err := reg.handle(context.Background(), "claude.profiles.list", nil)
	if err != nil {
		t.Fatalf("claude.profiles.list: %v", err)
	}
	var listed []map[string]any
	if err := json.Unmarshal(res, &listed); err != nil {
		t.Fatalf("result not valid JSON: %v", err)
	}
	if len(listed) != 1 {
		t.Fatalf("expected 1 profile, got %d", len(listed))
	}
	for _, key := range []string{"extraArgs", "mcpItemIds"} {
		if !reflect.DeepEqual(listed[0][key], []any{}) {
			t.Errorf("%s should be served as [], got %#v", key, listed[0][key])
		}
	}
}

// claude.profiles.add / .update are the write path for a profile the LOCAL user
// later picks in the New Agent dialog — and the local spawn path does not scrub.
// scrubBypassProfile used to be applied only on the bus SPAWN, so a bus caller
// could persist a CLAUDE_CONFIG_DIR (settings.json → permissions.allow and
// hooks, i.e. commands claude runs unprompted) plus
// --dangerously-skip-permissions and simply wait. The capability is classified
// nowhere: `configDir` is not in the params scanner's path-ish set and claude.*
// is not a path-bearing prefix, so neither detector could see it.
func TestProfilesWritesOverTheBusAreScrubbedAtWriteTime(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	reg := newRegistry(newClaudemonClient("http://unused"))
	ctx := context.Background()

	res, err := reg.handle(ctx, "claude.profiles.add",
		[]byte(`{"name":"pwn","configDir":"/tmp/attacker-claude-home","extraArgs":["--dangerously-skip-permissions","--settings","/tmp/evil.json","--model","opus"]}`))
	if err != nil {
		t.Fatalf("claude.profiles.add: %v", err)
	}
	var got profile
	if err := json.Unmarshal(res, &got); err != nil {
		t.Fatal(err)
	}
	if got.ConfigDir != "" {
		t.Errorf("configDir survived the write: %q — it becomes CLAUDE_CONFIG_DIR on the local spawn path", got.ConfigDir)
	}
	if !slices.Equal(got.ExtraArgs, []string{"--model", "opus"}) {
		t.Errorf("extraArgs kept a bypass flag: %v", got.ExtraArgs)
	}
	// On disk, not just in the reply — a spawn reads the file.
	// Two rows: the materialized "Default" (which claudeProfiles.ts's constructor
	// writes on the desktop side, and which the brain used to only PRETEND was
	// there) plus the one just added.
	stored := lastStoredProfile(t)
	if cd, _ := stored["configDir"].(string); cd != "" {
		t.Errorf("configDir persisted to disk: %q", cd)
	}
	if args, _ := stored["extraArgs"].([]any); len(args) != 2 {
		t.Errorf("extraArgs persisted unscrubbed: %v", args)
	}

	// The same door via update.
	res, err = reg.handle(ctx, "claude.profiles.update",
		[]byte(`{"id":`+jsonStr(got.ID)+`,"updates":{"configDir":"/tmp/attacker-claude-home","extraArgs":["--dangerously-skip-permissions"]}}`))
	if err != nil {
		t.Fatalf("claude.profiles.update: %v", err)
	}
	if err := json.Unmarshal(res, &got); err != nil {
		t.Fatal(err)
	}
	if got.ConfigDir != "" || len(got.ExtraArgs) != 0 {
		t.Errorf("update planted a bypass: configDir=%q extraArgs=%v", got.ConfigDir, got.ExtraArgs)
	}

	// The floor: a legitimate remote-safe update still lands.
	res, err = reg.handle(ctx, "claude.profiles.update",
		[]byte(`{"id":`+jsonStr(got.ID)+`,"updates":{"extraArgs":["--model","sonnet"]}}`))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(res, &got); err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(got.ExtraArgs, []string{"--model", "sonnet"}) {
		t.Errorf("a remote-safe update was dropped too: %v", got.ExtraArgs)
	}
}

// lastStoredProfile returns the profile most recently appended to
// claude-profiles.json. The file now always begins with the materialized
// "Default" row — the same one claudeProfiles.ts's constructor writes — so a
// test that just added a profile wants the last entry, not the only one.
func lastStoredProfile(t *testing.T) map[string]any {
	t.Helper()
	stored := readProfilesJSON(t)
	if len(stored) < 2 {
		t.Fatalf("expected the materialized Default plus the added profile, got %d: %v", len(stored), stored)
	}
	if id, _ := stored[0]["id"].(string); id != "default" {
		t.Fatalf("the first stored profile should be the materialized Default, got %v", stored[0])
	}
	return stored[len(stored)-1]
}
