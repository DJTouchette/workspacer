package main

import (
	"bytes"
	"log"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A CONFIG THAT VANISHED IS NOT A FIRST RUN.
//
// loadFromDisk already tells apart four ways a read can go wrong — unreadable,
// unparseable, empty, and a MID-RUN disappearance (c.current != nil) — and each
// blocks persistence rather than seeding over the user's settings. One arm was
// left silent: the very first read of the process. There, ENOENT means
// writeDefaults(), with nothing logged.
//
// That arm is the whole first-boot case, and on a machine whose state directory
// is a mounted volume it is also the every-boot case if the mount is wrong. The
// brain comes up healthy on a DIFFERENT configuration than the one it was left
// with: no projects, agents.binaries empty (so provider resolution silently
// falls back to PATH), supervisor.fullAccess off, budgets and keybindings gone.
//
// The evidence that separates the two is the directory around it. A genuine
// first run has nothing there. A config dir that still holds tokens.json,
// remote-token, sessions/ or layouts/ has been used, and a config.yaml missing
// from it was taken away.
func TestAConfigThatVanishedFromAnEstablishedDirIsReportedLoudly(t *testing.T) {
	home := tempConfigHome(t)
	dir := filepath.Join(home, "workspacer")
	if err := os.MkdirAll(filepath.Join(dir, "sessions"), 0o755); err != nil {
		t.Fatal(err)
	}
	// The rest of an established install, minus config.yaml.
	for _, f := range []string{"remote-token", "tokens.json"} {
		if err := os.WriteFile(filepath.Join(dir, f), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}

	var logged bytes.Buffer
	restore := log.Writer()
	log.SetOutput(&logged)
	defer log.SetOutput(restore)

	c := newConfigService()
	if c.get() == nil {
		t.Fatal("the brain must still come up: it is a supervised child, and exiting here " +
			"turns one missing file into a restart crash-loop that takes the node's whole " +
			"capability plane down")
	}

	out := logged.String()
	if !strings.Contains(out, "brain: STATE LOSS") {
		t.Errorf("config.yaml was reseeded with defaults and nothing said so.\nlog: %q", out)
	}
	if !strings.Contains(out, "config.yaml") {
		t.Errorf("log %q does not name the file", out)
	}
}

// The other half: a virgin config dir is a real first run and must stay quiet —
// a false alarm here would fire on every new install.
func TestAFirstRunOnAVirginConfigDirIsNotReportedAsLoss(t *testing.T) {
	tempConfigHome(t)

	var logged bytes.Buffer
	restore := log.Writer()
	log.SetOutput(&logged)
	defer log.SetOutput(restore)

	if newConfigService().get() == nil {
		t.Fatal("first run must seed defaults")
	}
	if strings.Contains(logged.String(), "STATE LOSS") {
		t.Errorf("first run reported as state loss: %q", logged.String())
	}
	if _, err := os.Stat(configPath()); err != nil {
		t.Errorf("first run must still write the seeded config.yaml: %v", err)
	}
}
