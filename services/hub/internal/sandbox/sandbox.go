// Package sandbox launches a plugin sidecar under OS-level filesystem
// confinement, so a sidecar that ignores the bus and pokes the OS directly still
// can't write outside its own directory. It complements the bus's capability
// scoping: the bus confines what a plugin asks workspacer to do; this confines
// what the sidecar process can do on its own.
//
// Scope of this layer, stated plainly:
//   - It confines FILESYSTEM WRITES to the plugin's own directory (+ a private
//     temp). Reads are left open — a sidecar still loads its interpreter and
//     libraries from the system — so this stops tampering/persistence, not
//     read-and-exfiltrate. A plugin that needs full confinement (e.g. the
//     editor) should ship as a *webview-only* plugin: no sidecar, nothing to
//     escape, the bus its only door.
//   - Network is NOT cut: every sidecar reaches the hub bus over loopback, so
//     isolating the network would break all of them.
//
// Mechanisms: bwrap (bubblewrap) on Linux, sandbox-exec (Seatbelt) on macOS.
// Both are command wrappers — we prepend them to the sidecar's argv, so the
// supervisor runs the wrapped command unchanged. Windows has no equivalent
// wrapper (FS confinement there needs AppContainer); it reports unavailable, and
// `enforce` mode fails closed.
//
// The security-relevant work — building the bwrap argv and the Seatbelt profile
// — lives in pure functions that are unit-tested on any platform. The only
// platform-specific bit is detecting whether the mechanism is present.
package sandbox

import (
	"os"
	"os/exec"
	"runtime"
	"strings"
)

// Mode controls what happens when a sandbox mechanism is or isn't available.
type Mode string

const (
	// ModeOff disables sandboxing — sidecars run as plain child processes.
	ModeOff Mode = "off"
	// ModeBestEffort sandboxes when a mechanism is available, and runs the
	// sidecar plain (with a warning) when it isn't. The default: never blocks a
	// plugin from running, adds confinement where the platform supports it.
	ModeBestEffort Mode = "best-effort"
	// ModeEnforce requires confinement: if no mechanism is available the sidecar
	// is refused (fail closed).
	ModeEnforce Mode = "enforce"
)

// ParseMode maps a string (e.g. from an env var) to a Mode, defaulting to
// best-effort for empty/unknown input.
func ParseMode(s string) Mode {
	switch Mode(strings.TrimSpace(s)) {
	case ModeOff:
		return ModeOff
	case ModeEnforce:
		return ModeEnforce
	default:
		return ModeBestEffort
	}
}

// Policy describes the confinement to apply to one sidecar.
type Policy struct {
	// WriteRoots are the only directories the sidecar may write to (its own
	// plugin dir; a private temp is always added on top). Reads are unrestricted.
	WriteRoots []string
}

// Result is the command to actually launch, plus whether real confinement was
// applied. When Available is false, Path/Args are the original command unchanged.
type Result struct {
	Path      string // argv[0] to exec
	Args      []string
	Available bool   // a real sandbox mechanism wrapped the command
	Mechanism string // "bwrap" | "sandbox-exec" | ""
	Note      string // why unavailable, for logs
}

// Decision is what the manager should do given a Mode and whether a mechanism
// was available.
type Decision int

const (
	// RunSandboxed: launch the wrapped (confined) command.
	RunSandboxed Decision = iota
	// RunUnsandboxed: launch the original command without confinement.
	RunUnsandboxed
	// Refuse: do not launch the sidecar at all.
	Refuse
)

// Decide resolves Mode + availability into an action. enforce never runs
// unconfined; off never confines; best-effort confines when it can.
func Decide(mode Mode, available bool) Decision {
	switch mode {
	case ModeOff:
		return RunUnsandboxed
	case ModeEnforce:
		if available {
			return RunSandboxed
		}
		return Refuse
	default: // best-effort
		if available {
			return RunSandboxed
		}
		return RunUnsandboxed
	}
}

// Wrap produces the launch command confining the given sidecar per policy. On a
// platform without a mechanism it returns the original command with Available
// false.
func Wrap(command string, args []string, p Policy) Result {
	switch runtime.GOOS {
	case "linux":
		return wrapLinux(command, args, p)
	case "darwin":
		return wrapDarwin(command, args, p)
	default:
		return Result{Path: command, Args: args, Note: "no filesystem sandbox mechanism on " + runtime.GOOS}
	}
}

// ── Linux: bubblewrap ────────────────────────────────────────────────────────

func wrapLinux(command string, args []string, p Policy) Result {
	bwrap, err := exec.LookPath("bwrap")
	if err != nil {
		return Result{Path: command, Args: args, Note: "bwrap (bubblewrap) not found in PATH"}
	}
	return Result{Path: bwrap, Args: buildBwrapArgs(command, args, p), Available: true, Mechanism: "bwrap"}
}

// buildBwrapArgs builds a bubblewrap argv that maps the whole filesystem
// read-only, then re-binds each write root (and a private tmp) read-write, so
// the sidecar can read the system + load its libraries but can only write inside
// its own directory. Network is shared (sidecars need loopback for the bus).
func buildBwrapArgs(command string, args []string, p Policy) []string {
	a := []string{
		"--die-with-parent",   // sidecar dies if the hub does — no orphans
		"--ro-bind", "/", "/", // read-only view of the host
		"--dev", "/dev",
		"--proc", "/proc",
		"--tmpfs", "/tmp", // private writable tmp
	}
	// Re-bind each write root read-write, overriding the read-only `/` above.
	for _, r := range p.WriteRoots {
		if r == "" {
			continue
		}
		a = append(a, "--bind", r, r)
	}
	a = append(a, "--")
	a = append(a, command)
	return append(a, args...)
}

// ── macOS: Seatbelt (sandbox-exec) ───────────────────────────────────────────

const sandboxExecPath = "/usr/bin/sandbox-exec"

func wrapDarwin(command string, args []string, p Policy) Result {
	if _, err := os.Stat(sandboxExecPath); err != nil {
		return Result{Path: command, Args: args, Note: "sandbox-exec not found"}
	}
	profile := buildSeatbeltProfile(p)
	wargs := append([]string{"-p", profile, command}, args...)
	return Result{Path: sandboxExecPath, Args: wargs, Available: true, Mechanism: "sandbox-exec"}
}

// buildSeatbeltProfile builds an SBPL profile that allows everything by default
// (so the process runs and reads what it needs), then denies all filesystem
// writes and re-allows them only under the write roots, a private temp, and the
// device nodes a typical process needs. Last matching rule wins in SBPL, so the
// re-allows override the blanket deny.
func buildSeatbeltProfile(p Policy) string {
	var b strings.Builder
	b.WriteString("(version 1)\n")
	b.WriteString("(allow default)\n")
	b.WriteString("(deny file-write*)\n")
	b.WriteString("(allow file-write*\n")
	for _, r := range p.WriteRoots {
		if r == "" {
			continue
		}
		b.WriteString("  (subpath ")
		b.WriteString(sbplString(r))
		b.WriteString(")\n")
	}
	// Temp + the device nodes processes commonly need to write.
	b.WriteString("  (subpath \"/private/tmp\")\n")
	b.WriteString("  (subpath \"/private/var/folders\")\n") // $TMPDIR
	b.WriteString("  (literal \"/dev/null\")\n")
	b.WriteString("  (literal \"/dev/dtracehelper\")\n")
	b.WriteString("  (regex #\"^/dev/tty\")\n")
	b.WriteString(")\n")
	return b.String()
}

// sbplString renders a Go string as an SBPL double-quoted literal.
func sbplString(s string) string {
	s = strings.ReplaceAll(s, "\\", "\\\\")
	s = strings.ReplaceAll(s, "\"", "\\\"")
	return "\"" + s + "\""
}
