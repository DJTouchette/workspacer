package main

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

// waitUntil polls cond until it's true or the deadline passes. Poll-based so
// the process tests stay robust under -race and loaded CI machines.
func waitUntil(timeout time.Duration, cond func() bool) bool {
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if cond() {
			return true
		}
		time.Sleep(20 * time.Millisecond)
	}
	return cond()
}

// spawnCount counts how many times the fake child ran (one line per spawn).
func spawnCount(file string) int {
	b, err := os.ReadFile(file)
	if err != nil {
		return 0
	}
	return strings.Count(string(b), "\n")
}

// fastBackoff shrinks the real policy so restart tests finish in milliseconds.
func fastBackoff(maxAttempts int) *restartBackoff {
	bo := newRestartBackoff()
	bo.base = 5 * time.Millisecond
	bo.max = 20 * time.Millisecond
	bo.maxAttempts = maxAttempts
	return bo
}

func requireSh(t *testing.T) {
	t.Helper()
	if runtime.GOOS == "windows" {
		t.Skip("fake child uses /bin/sh")
	}
}

func TestChildRestartsOnCrash(t *testing.T) {
	requireSh(t)
	cnt := filepath.Join(t.TempDir(), "count")
	spec := childSpec{
		Name: "crashy",
		Bin:  "/bin/sh",
		Args: []string{"-c", "echo x >> " + cnt + "; exit 1"},
	}
	c := startChild(context.Background(), spec, io.Discard, fastBackoff(10))
	defer c.Stop()

	if !waitUntil(5*time.Second, func() bool { return spawnCount(cnt) >= 3 }) {
		t.Fatalf("child was not restarted after crashing (spawned %d times)", spawnCount(cnt))
	}
}

func TestChildGivesUpAfterBudget(t *testing.T) {
	requireSh(t)
	cnt := filepath.Join(t.TempDir(), "count")
	spec := childSpec{
		Name: "hopeless",
		Bin:  "/bin/sh",
		Args: []string{"-c", "echo x >> " + cnt + "; exit 1"},
	}
	c := startChild(context.Background(), spec, io.Discard, fastBackoff(2))
	defer c.Stop()

	if !waitUntil(5*time.Second, c.GaveUp) {
		t.Fatal("child never gave up despite exhausting the restart budget")
	}
	// Initial spawn + 2 restarts = 3 runs, then nothing more.
	if got := spawnCount(cnt); got != 3 {
		t.Errorf("spawned %d times, want 3 (1 initial + budget of 2 restarts)", got)
	}
	final := spawnCount(cnt)
	time.Sleep(100 * time.Millisecond)
	if got := spawnCount(cnt); got != final {
		t.Errorf("child respawned after giving up (%d → %d)", final, got)
	}
}

func TestChildCleanStopDoesNotRestart(t *testing.T) {
	requireSh(t)
	cnt := filepath.Join(t.TempDir(), "count")
	spec := childSpec{
		Name: "steady",
		Bin:  "/bin/sh",
		Args: []string{"-c", "echo x >> " + cnt + "; exec sleep 30"},
	}
	c := startChild(context.Background(), spec, io.Discard, fastBackoff(10))

	if !waitUntil(5*time.Second, func() bool { return spawnCount(cnt) == 1 }) {
		t.Fatal("child never started")
	}
	start := time.Now()
	c.Stop() // SIGTERM should end the sleep well before WaitDelay's SIGKILL
	if elapsed := time.Since(start); elapsed > 8*time.Second {
		t.Errorf("Stop took %v — SIGTERM path did not work", elapsed)
	}
	if got := spawnCount(cnt); got != 1 {
		t.Errorf("intentional stop triggered a restart (spawned %d times)", got)
	}
	if c.GaveUp() {
		t.Error("clean stop reported as give-up")
	}
}

func TestChildParentContextCancelStops(t *testing.T) {
	requireSh(t)
	cnt := filepath.Join(t.TempDir(), "count")
	ctx, cancel := context.WithCancel(context.Background())
	spec := childSpec{
		Name: "ctxed",
		Bin:  "/bin/sh",
		Args: []string{"-c", "echo x >> " + cnt + "; exec sleep 30"},
	}
	c := startChild(ctx, spec, io.Discard, fastBackoff(10))
	if !waitUntil(5*time.Second, func() bool { return spawnCount(cnt) == 1 }) {
		t.Fatal("child never started")
	}
	cancel() // the serve signal context — Ctrl-C path
	select {
	case <-c.done:
	case <-time.After(10 * time.Second):
		t.Fatal("supervision loop did not exit on context cancel")
	}
	if got := spawnCount(cnt); got != 1 {
		t.Errorf("context cancel triggered a restart (spawned %d times)", got)
	}
}

func TestPrefixWriter(t *testing.T) {
	tests := []struct {
		name   string
		writes []string
		want   string
	}{
		{"single line", []string{"hello\n"}, "[x] hello\n"},
		{"two lines in one write", []string{"a\nb\n"}, "[x] a\n[x] b\n"},
		{"line split across writes", []string{"par", "tial\n"}, "[x] partial\n"},
		{"trailing partial flushed", []string{"no newline"}, "[x] no newline\n"},
		{"empty write emits nothing", []string{""}, ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var sb strings.Builder
			pw := &prefixWriter{w: &sb, prefix: "[x] "}
			for _, w := range tt.writes {
				if _, err := pw.Write([]byte(w)); err != nil {
					t.Fatal(err)
				}
			}
			pw.flush()
			if sb.String() != tt.want {
				t.Errorf("output = %q, want %q", sb.String(), tt.want)
			}
		})
	}
}
