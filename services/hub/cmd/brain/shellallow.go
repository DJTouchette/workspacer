package main

// The `shell` param of terminals.create is argv[0] of a process this daemon
// spawns on the host, taken verbatim from a bus caller.
//
// capspec leaves terminals.create unscoped, and its recorded reason named ONE
// param: "cwd is a process working directory, and holding the capability at all
// is the gate". That was incomplete in exactly the way sessions.*'s silence was —
// `shell` is a second, unnamed process identifier, it is not in the params
// scanner's path-ish set, and neither provider checked it at all. Combined with a
// mode-preserving fs.write over an existing executable inside the caller's own
// agent cwd (os.WriteFile and writeFileSync both keep the 0755), terminals.create
// alone was arbitrary host code execution.
//
// The fix is an ALLOWLIST rather than containment, because there is no subtree we
// could confine this to that the same caller cannot also fill in — the same
// argument scrubBypassProfile makes about CLAUDE_CONFIG_DIR. A shell is one of
// the login shells the host already trusts: what the user's own $SHELL says, what
// /etc/shells lists, and the platform fallbacks. Anything else is refused rather
// than reasoned about, so a caller cannot nominate a launcher it just wrote.
//
// TWIN: apps/desktop/src/main/lib/shellAllowlist.ts.

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// fallbackShells are the platform's own defaults, always allowed. They are what
// terminals.create used when the caller named nothing, so refusing them would
// refuse the call it has always made.
func fallbackShells() []string {
	if runtime.GOOS == "windows" {
		return []string{"powershell.exe", "pwsh.exe", "cmd.exe"}
	}
	return []string{"/bin/sh", "/bin/bash", "/bin/zsh", "/usr/bin/bash", "/usr/bin/zsh", "/bin/fish", "/usr/bin/fish"}
}

// etcShellsPath is a var so a test can point it at a fixture.
var etcShellsPath = "/etc/shells"

// allowedShells is the set a caller may name: $SHELL, /etc/shells, and the
// platform fallbacks. Read at call time — a user who installs a new shell should
// not have to restart the daemon.
func allowedShells() map[string]bool {
	set := map[string]bool{}
	add := func(s string) {
		s = strings.TrimSpace(s)
		if s == "" || strings.HasPrefix(s, "#") {
			return
		}
		set[s] = true
	}
	for _, s := range fallbackShells() {
		add(s)
	}
	add(os.Getenv("SHELL"))
	if raw, err := os.ReadFile(etcShellsPath); err == nil {
		for _, line := range strings.Split(string(raw), "\n") {
			add(line)
		}
	}
	return set
}

// resolveTerminalShell answers "what argv[0] does this terminals.create get".
// An empty request takes the host default; a named one must be on the allowlist.
// ok=false means refuse the call — never "fall back to the default", which would
// hide the refusal from the caller and make the allowlist untestable from the
// outside.
func resolveTerminalShell(requested string) (string, bool) {
	def := os.Getenv("SHELL")
	if def == "" {
		def = fallbackShells()[0]
	}
	if strings.TrimSpace(requested) == "" {
		return def, true
	}
	if allowedShells()[requested] {
		return requested, true
	}
	// On Windows the caller may name a bare command; compare on the basename so
	// "powershell.exe" matches without demanding the caller know its location.
	if runtime.GOOS == "windows" {
		base := strings.ToLower(filepath.Base(requested))
		for s := range allowedShells() {
			if strings.ToLower(filepath.Base(s)) == base {
				return requested, true
			}
		}
	}
	return "", false
}
