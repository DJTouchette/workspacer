package main

import (
	"fmt"
	"log"
	"os"
	"path/filepath"
	"time"
)

// Cross-process advisory lock for config.yaml.
//
// config.yaml has two writers — this brain (config.save over the bus, which is
// what the web and mobile Settings panes use) and the desktop's configService
// (in process, for its own Settings pane). Both do refresh-if-changed →
// deepMerge → atomic write, and the mtime gate each of them has closes only the
// first step. Nothing spans the three, so an interleaved write from the other
// process is silently lost and both report success.
//
// Neither process can simply own the file: headless `workspacer serve` runs with
// no Electron, and this brain is itself optional (the hub serves brain-less when
// no binary is found). So the file arbitrates. See contracts/config-lock.json;
// the TS twin is apps/desktop/src/main/lib/configLock.ts.

const (
	// Beside the file it guards, so it shares its filesystem and permissions.
	lockFileSuffix = ".lock"
	// A lock older than this is treated as abandoned and stolen.
	//
	// MUST match the TS twin. A side that expires locks sooner than the other
	// will steal one the other still believes it holds — worse than no lock,
	// because both then write believing they are exclusive.
	lockStaleMs = 10_000
	// This is a background process, so it can afford to wait longer than the
	// desktop, whose saveConfig blocks the Electron main thread. Wait budgets are
	// deliberately per-side; only the stale threshold is a correctness parameter.
	lockMaxWait = 2000 * time.Millisecond
	lockRetry   = 10 * time.Millisecond
)

// errConfigLocked is returned when the lock could not be taken in time. The
// caller must treat it as a failed save rather than writing anyway.
var errConfigLocked = fmt.Errorf("config.yaml is locked by another process")

// withConfigLock runs fn holding the lock for path.
func withConfigLock(path string, fn func() error) error {
	lockPath := path + lockFileSuffix
	// The lock lives beside the file it guards, and on a fresh install that
	// directory does not exist yet — the writer creates it, and the writer runs
	// INSIDE the lock. Without this the very first save on a new machine fails to
	// acquire and is refused.
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	deadline := time.Now().Add(lockMaxWait)

	for {
		// O_EXCL is atomic "create only if absent" against both the other process
		// and another goroutine here.
		f, err := os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o644)
		if err == nil {
			// Diagnostics only; holding the lock is what matters.
			_, _ = fmt.Fprintf(f, "%d %s\n", os.Getpid(), time.Now().UTC().Format(time.RFC3339))
			_ = f.Close()
			break
		}
		if !os.IsExist(err) {
			return err
		}
		// Held. Steal it if the holder died mid-write, or wait a moment.
		if lockIsStale(lockPath) {
			// A failed remove means another waiter got there first; the retry
			// below re-races fairly.
			_ = os.Remove(lockPath)
			continue
		}
		if time.Now().After(deadline) {
			return errConfigLocked
		}
		time.Sleep(lockRetry)
	}

	defer func() {
		// A failed release would wedge config until lockStaleMs; nothing better
		// to do than say so.
		if err := os.Remove(lockPath); err != nil && !os.IsNotExist(err) {
			log.Printf("brain: could not release the config lock %s: %v", lockPath, err)
		}
	}()
	return fn()
}

// lockIsStale reports whether the lock file is old enough that its holder must
// have died.
func lockIsStale(lockPath string) bool {
	st, err := os.Stat(lockPath)
	if err != nil {
		// Vanished between the O_EXCL failure and the stat — the holder released
		// it, so it is not stale, it is gone. Retrying will take it.
		return false
	}
	return time.Since(st.ModTime()) > lockStaleMs*time.Millisecond
}
