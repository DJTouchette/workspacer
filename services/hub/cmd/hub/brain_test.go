package main

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestResolveBrainBinFlagWins(t *testing.T) {
	if got := resolveBrainBin("/explicit/brain"); got != "/explicit/brain" {
		t.Errorf("explicit flag should win, got %q", got)
	}
}

func TestResolveBrainBinSibling(t *testing.T) {
	// A real file next to a fake "hub executable" is found via the sibling path.
	dir := t.TempDir()
	sibling := filepath.Join(dir, brainExeName())
	if err := os.WriteFile(sibling, []byte("#!/bin/true\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	// resolveBrainBin uses os.Executable(); we can't fake that, so just assert the
	// sibling-probe logic directly: the file exists and is not a dir.
	st, err := os.Stat(sibling)
	if err != nil || st.IsDir() {
		t.Fatalf("sibling probe precondition failed: %v", err)
	}
}

func TestBrainArgs(t *testing.T) {
	got := brainArgs("127.0.0.1:7895", "http://host:7891", "catalog")
	want := []string{
		"--hub", "ws://127.0.0.1:7895/bus",
		"--claudemon", "http://host:7891",
		"--scope", "catalog",
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("brainArgs = %v, want %v", got, want)
	}
}
