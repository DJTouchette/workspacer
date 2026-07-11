package main

import (
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
)

// runInstallCLI puts this binary on PATH so `workspacer serve` works from any
// shell. Unix prefers a symlink (upgrades of the real binary are picked up for
// free); Windows gets a copy (symlinks need privileges there) plus PATH
// instructions — the codebase has no registry-PATH-editing precedent
// (installs are electron-builder's job), so we don't invent one here.
func runInstallCLI(args []string) int {
	fs := flag.NewFlagSet("workspacer install-cli", flag.ExitOnError)
	dir := fs.String("dir", "", "install directory (default: /usr/local/bin, then ~/.local/bin; %LOCALAPPDATA%\\workspacer\\bin on Windows)")
	_ = fs.Parse(args)

	self, err := os.Executable()
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: cannot locate this executable: %v\n", err)
		return 1
	}
	if real, err := filepath.EvalSymlinks(self); err == nil {
		self = real
	}

	target := *dir
	if target == "" {
		home, _ := os.UserHomeDir()
		target = pickInstallDir(runtime.GOOS, home, os.Getenv("LOCALAPPDATA"), dirWritable)
	}
	if err := os.MkdirAll(target, 0o755); err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: cannot create %s: %v\n", target, err)
		return 1
	}

	dst := filepath.Join(target, exeName("workspacer"))
	if err := installBinary(self, dst); err != nil {
		fmt.Fprintf(os.Stderr, "workspacer: install to %s failed: %v\n", dst, err)
		return 1
	}
	fmt.Printf("installed %s -> %s\n", dst, self)

	if !onPath(target, os.Getenv("PATH")) {
		fmt.Printf("\n%s is not on your PATH. Add it:\n", target)
		if runtime.GOOS == "windows" {
			fmt.Printf("  Settings > System > About > Advanced system settings > Environment Variables\n  (or in PowerShell: [Environment]::SetEnvironmentVariable('Path', $env:Path + ';%s', 'User'))\n", target)
		} else {
			fmt.Printf("  echo 'export PATH=\"%s:$PATH\"' >> ~/.profile   # or your shell's rc file\n", target)
		}
	}
	return 0
}

// pickInstallDir chooses where the binary lands. Unix: the system-wide
// /usr/local/bin when we can actually write there (running as root / admin
// setups), else the user-scoped ~/.local/bin (XDG's conventional user bin —
// no sudo prompts from a CLI). Windows: the per-user app dir; putting a copy
// under Program Files would need elevation. Pure over `writable` for tests.
func pickInstallDir(goos, home, localAppData string, writable func(string) bool) string {
	if goos == "windows" {
		if localAppData == "" {
			localAppData = filepath.Join(home, "AppData", "Local")
		}
		return filepath.Join(localAppData, "workspacer", "bin")
	}
	if writable("/usr/local/bin") {
		return "/usr/local/bin"
	}
	return filepath.Join(home, ".local", "bin")
}

// dirWritable reports whether we can create a file in dir, by trying — a
// permission-bit check would miss ACLs, read-only mounts, and containers.
func dirWritable(dir string) bool {
	f, err := os.CreateTemp(dir, ".workspacer-install-*")
	if err != nil {
		return false
	}
	name := f.Name()
	_ = f.Close()
	_ = os.Remove(name)
	return true
}

// installBinary links (preferred) or copies src to dst, replacing what's
// there. The symlink keeps future `make build-cli` outputs live without
// re-running install-cli; copying is the fallback for filesystems/platforms
// where linking fails.
func installBinary(src, dst string) error {
	if src == dst {
		return nil // installing over ourselves — already there
	}
	// Remove the old entry first: symlink/rename onto an existing file fails,
	// and on Windows an in-use exe can't be overwritten but CAN be renamed away.
	if _, err := os.Lstat(dst); err == nil {
		if err := os.Remove(dst); err != nil {
			if renameErr := os.Rename(dst, dst+".old"); renameErr != nil {
				return fmt.Errorf("cannot replace existing %s: %w", dst, err)
			}
		}
	}
	if runtime.GOOS != "windows" {
		if err := os.Symlink(src, dst); err == nil {
			return nil
		}
	}
	return copyFile(src, dst)
}

// copyFile copies src to dst with the executable bit set.
func copyFile(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o755)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}
	return out.Close()
}

// onPath reports whether dir is one of the PATH entries.
func onPath(dir, pathEnv string) bool {
	for _, p := range filepath.SplitList(pathEnv) {
		if p == dir {
			return true
		}
	}
	return false
}
