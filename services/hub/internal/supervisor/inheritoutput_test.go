package supervisor

// A supervised child whose stdout is discarded writes its diagnosis to nowhere.
// That is exactly how the brain's 403 reconnect loop stayed invisible: its Spec
// had neither LogLines nor InheritOutput, so `cmd.Stdout` was nil and Go threw
// every byte away. The fix is one field, and until now nothing in either stack
// asserted that a supervised child's output reaches anywhere at all — so it
// could be reverted with a fully green tree and the next permanent failure
// would be silent again.

import (
	"os"
	"runtime"
	"strings"
	"testing"
	"time"
)

// captureStdout swaps os.Stdout for a pipe and returns everything written to it
// while fn runs. The supervisor wires cmd.Stdout to os.Stdout by value at spawn
// time, so the swap must be in place before Start.
func captureStdout(t *testing.T, fn func()) string {
	t.Helper()
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	prev := os.Stdout
	os.Stdout = w
	done := make(chan string, 1)
	go func() {
		buf := make([]byte, 4096)
		var sb strings.Builder
		for {
			n, err := r.Read(buf)
			if n > 0 {
				sb.Write(buf[:n])
			}
			if err != nil {
				break
			}
		}
		done <- sb.String()
	}()
	fn()
	os.Stdout = prev
	_ = w.Close()
	out := <-done
	_ = r.Close()
	return out
}

func TestInheritOutputForwardsASupervisedChildsDiagnosis(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("uses /bin/sh")
	}
	const marker = "BRAIN-403-LOOP-EXPLAINED-HERE"

	got := captureStdout(t, func() {
		s := New(Spec{
			Name:          "brain",
			Command:       "/bin/sh",
			Args:          []string{"-c", "echo " + marker},
			InheritOutput: true,
		}, nil)
		s.Start()
		time.Sleep(400 * time.Millisecond) // let the child run and the pipe drain
		s.Stop()
	})

	if !strings.Contains(got, marker) {
		t.Fatalf("a supervised child's stdout was DISCARDED: a permanent failure writes its diagnosis to nowhere.\ngot: %q", got)
	}
}

// The default stays quiet: InheritOutput is an opt-in, so a Spec that does not
// ask for it must not start dumping every sidecar's chatter into the hub's log.
func TestOutputIsStillDiscardedWithoutInheritOutput(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("uses /bin/sh")
	}
	const marker = "SIDECAR-CHATTER-NOBODY-ASKED-FOR"

	got := captureStdout(t, func() {
		s := New(Spec{
			Name:    "sidecar",
			Command: "/bin/sh",
			Args:    []string{"-c", "echo " + marker},
		}, nil)
		s.Start()
		time.Sleep(400 * time.Millisecond)
		s.Stop()
	})

	if strings.Contains(got, marker) {
		t.Fatalf("output was inherited without InheritOutput being set: %q", got)
	}
}
