package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// terminals.create's `shell` is argv[0] of a host process, straight from a bus
// caller. Nothing checked it in either provider, and capspec's own record for
// this capability named only `cwd`.
func TestTerminalShellIsAnAllowlistNotAPassthrough(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("the /etc/shells fixture below is POSIX-shaped")
	}
	dir := t.TempDir()
	shells := filepath.Join(dir, "shells")
	if err := os.WriteFile(shells, []byte("# comment\n/bin/bash\n/usr/local/bin/xonsh\n\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	restore := etcShellsPath
	etcShellsPath = shells
	t.Cleanup(func() { etcShellsPath = restore })
	t.Setenv("SHELL", "/bin/zsh")

	// The floor: a real login shell, in each of the three ways one qualifies.
	for _, ok := range []string{"/bin/zsh" /* $SHELL */, "/bin/sh" /* fallback */, "/usr/local/bin/xonsh" /* /etc/shells */} {
		got, allowed := resolveTerminalShell(ok)
		if !allowed || got != ok {
			t.Errorf("resolveTerminalShell(%q) = (%q, %v); a host login shell must be allowed", ok, got, allowed)
		}
	}
	// An empty request takes the host default rather than failing.
	if got, allowed := resolveTerminalShell(""); !allowed || got != "/bin/zsh" {
		t.Errorf(`resolveTerminalShell("") = (%q, %v), want ("/bin/zsh", true)`, got, allowed)
	}

	// The point: an arbitrary executable — including one the same caller could
	// have just written into its own agent cwd with fs.write, which preserves the
	// 0755 mode of an existing file — is refused, not silently downgraded.
	planted := filepath.Join(dir, "node_modules", ".bin", "tsc")
	if err := os.MkdirAll(filepath.Dir(planted), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(planted, []byte("#!/bin/sh\nid > /tmp/pwned\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{planted, "/usr/bin/env", "/bin/sh -c id", "../../bin/sh", "sh"} {
		if got, allowed := resolveTerminalShell(bad); allowed {
			t.Errorf("resolveTerminalShell(%q) = (%q, true); an arbitrary executable is not a login shell", bad, got)
		}
	}
	// A refusal must be a refusal, not "use the default anyway" — otherwise the
	// allowlist is invisible to the caller and untestable from outside.
	if got, allowed := resolveTerminalShell(planted); allowed || got != "" {
		t.Errorf("a refused shell returned (%q, %v); want (\"\", false)", got, allowed)
	}

	// A comment line in /etc/shells is not a shell.
	if _, allowed := resolveTerminalShell("# comment"); allowed {
		t.Error("a comment line from /etc/shells was treated as a shell")
	}
}

// The handler, not only the helper. resolveTerminalShell can be right and the
// call site can still pass p.Shell straight through — which is exactly what
// shipped, with capspec's record for terminals.create naming only `cwd`.
func TestTerminalsCreateRefusesAShellThatIsNotOne(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("the /etc/shells fixture is POSIX-shaped")
	}
	dir := t.TempDir()
	shells := filepath.Join(dir, "shells")
	if err := os.WriteFile(shells, []byte("/bin/bash\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	restore := etcShellsPath
	etcShellsPath = shells
	t.Cleanup(func() { etcShellsPath = restore })
	t.Setenv("SHELL", "/bin/zsh")

	// A launcher the caller could have written into its own agent cwd with
	// fs.write, which preserves the 0755 mode of an existing file.
	planted := filepath.Join(dir, "node_modules", ".bin", "tsc")
	if err := os.MkdirAll(filepath.Dir(planted), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(planted, []byte("#!/bin/sh\nid > /tmp/pwned\n"), 0o755); err != nil {
		t.Fatal(err)
	}

	// No claudemon: a REFUSAL must happen before the spawn is attempted, so the
	// error must name the shell rather than a connection failure.
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	_, err := reg.handle(context.Background(), "terminals.create",
		json.RawMessage(`{"shell":`+jsonStr(planted)+`,"cwd":`+jsonStr(dir)+`}`))
	if err == nil {
		t.Fatal("terminals.create accepted an arbitrary executable as argv[0]")
	}
	if !strings.Contains(err.Error(), "login shell") {
		t.Fatalf("refused for the wrong reason (the spawn was attempted first?): %v", err)
	}
}
