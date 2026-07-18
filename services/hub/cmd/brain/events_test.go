package main

import (
	"testing"
	"time"
)

// TestBackoffAfterConnResetsAfterLongLivedConnection pins the SSE reconnect
// backoff-reset behavior that runSessionStore/runStatusLines must share with
// busclient.Run: a stream that stayed up a while is a fresh failure, not a
// tight loop, so the escalating backoff resets to the base delay.
func TestBackoffAfterConnResetsAfterLongLivedConnection(t *testing.T) {
	// Backoff already climbed to the 10s cap, then a long-lived connection
	// dropped: the next reconnect must be prompt (base 1s), not the full cap.
	if got := backoffAfterConn(10*time.Second, 30*time.Second); got != time.Second {
		t.Fatalf("long-lived connection should reset backoff to 1s, got %v", got)
	}
	// A connection that dropped almost immediately keeps the escalating backoff.
	if got := backoffAfterConn(10*time.Second, 500*time.Millisecond); got != 10*time.Second {
		t.Fatalf("short-lived connection should preserve backoff, got %v", got)
	}
	// Boundary: just over the 5s threshold resets.
	if got := backoffAfterConn(4*time.Second, 6*time.Second); got != time.Second {
		t.Fatalf("connection past reset threshold should reset to 1s, got %v", got)
	}
}
