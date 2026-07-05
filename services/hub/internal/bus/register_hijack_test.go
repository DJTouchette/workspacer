package bus

import (
	"testing"
	"time"
)

// registerMethods sends a register frame and returns the methods the hub
// actually accepted (its `registered` ack reflects ownership decisions).
func (c *client) registerMethods(methods ...string) []string {
	c.t.Helper()
	c.send(Frame{Op: "register", Methods: methods})
	return c.readUntil("registered").Methods
}

func contains(ss []string, want string) bool {
	for _, s := range ss {
		if s == want {
			return true
		}
	}
	return false
}

// A second live connection cannot re-register a capability another live
// connection already owns — the hijack the guard prevents.
func TestRegisterHijackRefused(t *testing.T) {
	url := rpcServer(t)
	owner := dialClient(t, url)
	attacker := dialClient(t, url)

	if got := owner.registerMethods("agents.spawn"); !contains(got, "agents.spawn") {
		t.Fatalf("owner should have registered agents.spawn, got %v", got)
	}
	if got := attacker.registerMethods("agents.spawn"); contains(got, "agents.spawn") {
		t.Fatalf("attacker must not hijack agents.spawn, but ack was %v", got)
	}

	// The owner re-registering its own method is idempotent, not a self-hijack.
	if got := owner.registerMethods("agents.spawn"); !contains(got, "agents.spawn") {
		t.Fatalf("owner re-registering its own method should succeed, got %v", got)
	}
}

// Once the owner's connection drops, the capability is free for another
// connection to claim — this is what makes the desktop's reconnect work.
func TestRegisterAllowedAfterOwnerDisconnect(t *testing.T) {
	url := rpcServer(t)
	owner := dialClient(t, url)
	if got := owner.registerMethods("agents.spawn"); !contains(got, "agents.spawn") {
		t.Fatalf("owner should have registered agents.spawn, got %v", got)
	}

	// Drop the owner; the server's dropConn releases its providers.
	owner.ws.CloseNow()

	successor := dialClient(t, url)
	// dropConn runs when the server observes the close; poll briefly so the test
	// doesn't race the teardown.
	deadline := time.Now().Add(2 * time.Second)
	for {
		if contains(successor.registerMethods("agents.spawn"), "agents.spawn") {
			return // reclaimed — success
		}
		if time.Now().After(deadline) {
			t.Fatal("successor could not reclaim agents.spawn after owner disconnected")
		}
		time.Sleep(20 * time.Millisecond)
	}
}
