package main

// Reading the node's own record of how its PREVIOUS run ended.
//
// WHY THE HUB CANNOT JUST ASK THE CLOUD API. Fly's `on-failure` restart policy
// retries a crashing machine and then leaves it `stopped` — which through the
// Machines API is byte-for-byte identical to a machine the operator put to
// sleep on purpose. Those two look the same and mean opposite things, and a
// registry that renders the first as "asleep and fine" is exactly the quiet
// wrongness a remote node must not have.
//
// The node's entrypoint (deploy/fly/node/entrypoint.sh) closes that gap from
// the only side that can see it: it writes {bootId, reason, exitCode, at,
// machine} to $WKS_DATA/state/last-exit.json on every exit. `signal-TERM` is a
// deliberate stop; `claudemon-died`, `brain-died` and `boot-failure` are not.
//
// THE HONEST LIMIT, and it is worth stating rather than discovering: this file
// lives on the NODE's volume, so the hub cannot read it while the node is off
// — which is precisely when the question is asked. What it can do is report
// the previous run's ending on the NEXT attachment, which is how a crash that
// happened after a healthy start becomes visible at all. The crash-LOOP case
// (a node that never gets far enough to register) is caught from the hub side
// instead, by the wake that times out.
//
// It rides on brain.info rather than being its own capability: brain.info is
// already the liveness probe the hub calls on a timer, so this costs no extra
// call, no new method, and no new entry in four registries.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

// exitRecord is the node's own record of how its previous run ended.
type exitRecord struct {
	Reason   string `json:"reason,omitempty"`
	ExitCode *int   `json:"exitCode,omitempty"`
	At       string `json:"at,omitempty"`
}

var (
	lastExitOnce sync.Once
	lastExitVal  *exitRecord
)

// lastExitPath is where the node's entrypoint writes the record. Derived from
// WKS_DATA, which the entrypoint exports; unset everywhere else, which is what
// keeps this inert on a desktop or a laptop.
func lastExitPath() string {
	dir := strings.TrimSpace(os.Getenv("WKS_DATA"))
	if dir == "" {
		return ""
	}
	return filepath.Join(dir, "state", "last-exit.json")
}

// lastExit reads the record once and caches it.
//
// Once is correct rather than lazy: the file is written by the entrypoint when
// the process tree EXITS, so it cannot change while this brain is alive. A
// re-read on every liveness probe would be a syscall every thirty seconds for
// a value that is fixed for the life of the process.
func lastExit() *exitRecord {
	lastExitOnce.Do(func() { lastExitVal = readExitRecord(lastExitPath()) })
	return lastExitVal
}

// readExitRecord parses one record, or returns nil.
//
// A missing, unreadable or nonsense file is NO RECORD, never a fabricated
// empty one: "nobody knows how it ended" and "it ended cleanly" are different
// answers and only one of them should stop a person looking.
func readExitRecord(path string) *exitRecord {
	if path == "" {
		return nil
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return nil // first boot, no volume, or not a node — all "no record"
	}
	var rec exitRecord
	if err := json.Unmarshal(b, &rec); err != nil || rec.Reason == "" {
		return nil
	}
	return &rec
}
