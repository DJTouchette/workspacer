package main

import (
	"slices"
	"strings"
	"testing"
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
	argv := buildArgv(&profile{}, "", false, "", "abc-123", true)
	if !hasPair(argv, "--resume", "abc-123") {
		t.Fatalf("expected --resume abc-123, got %v", argv)
	}
	if slices.Contains(argv, "--session-id") {
		t.Fatalf("resume must not add --session-id, got %v", argv)
	}
}

func TestFreshSpawnUsesSessionIDNotResume(t *testing.T) {
	argv := buildArgv(&profile{}, "", false, "", "abc-123", false)
	if !hasPair(argv, "--session-id", "abc-123") {
		t.Fatalf("expected --session-id abc-123, got %v", argv)
	}
	if slices.Contains(argv, "--resume") {
		t.Fatalf("fresh spawn must not add --resume, got %v", argv)
	}
}

func TestProfilePinnedModelNotDuplicated(t *testing.T) {
	p := &profile{ExtraArgs: []string{"--model", "opus"}}
	argv := buildArgv(p, "sonnet", false, "", "", false)
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
	argv := buildArgv(&profile{}, "", true, "", "", false)
	if !slices.Contains(argv, "--dangerously-skip-permissions") {
		t.Fatalf("expected skip-permissions flag, got %v", argv)
	}
	// Not added when the profile already pins it.
	p := &profile{ExtraArgs: []string{"--dangerously-skip-permissions"}}
	argv = buildArgv(p, "", true, "", "", false)
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
	argv := buildArgv(&profile{}, "", false, "plan", "", false)
	if !hasPair(argv, "--permission-mode", "plan") {
		t.Fatalf("expected --permission-mode plan, got %v", argv)
	}
	// 'default' adds no flag.
	argv = buildArgv(&profile{}, "", false, "default", "", false)
	if slices.Contains(argv, "--permission-mode") {
		t.Fatalf("'default' must not add --permission-mode, got %v", argv)
	}
	// 'bypassPermissions' rides the skip flag, never --permission-mode.
	argv = buildArgv(&profile{}, "", false, "bypassPermissions", "", false)
	if !slices.Contains(argv, "--dangerously-skip-permissions") || slices.Contains(argv, "--permission-mode") {
		t.Fatalf("bypass must map to the skip flag only, got %v", argv)
	}
	// A profile-pinned mode wins over the requested one.
	p := &profile{ExtraArgs: []string{"--permission-mode", "acceptEdits"}}
	argv = buildArgv(p, "", false, "plan", "", false)
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
	argv := buildArgv(p, "", false, "", "", false)
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
