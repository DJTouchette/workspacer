package bus

import (
	"testing"
	"time"
)

// THE ZOMBIE PROVIDER.
//
// A remote node's brain is a provider connection that never subscribes to a
// topic, so the hub never writes to it unprompted and a failed write can never
// reveal that the far end is gone. If the machine it runs on stops without the
// TCP connection being severed cleanly — which is exactly what a stopped cloud
// VM may do — the hub's read loop never returns, dropConn never runs, and the
// registration slot stays owned by a connection that will never answer again.
//
// The consequence is the one that makes a wake feel broken: the machine boots,
// its brain dials in, re-registers every capability, and is REFUSED all of
// them by the first-registration-wins guard. The node is up and provides
// nothing.
//
// This test pins the failure so the fix has something to be measured against.
func TestStaleProviderKeepsItsSlotAndBlocksTheSuccessor(t *testing.T) {
	url := rpcServer(t)
	zombie := dialClient(t, url)
	if got := zombie.registerMethods("brain.info"); !contains(got, "brain.info") {
		t.Fatalf("first provider should own brain.info, got %v", got)
	}

	// The far end is gone but the socket is not closed: we simply never answer
	// anything on it again. From the hub's side this is indistinguishable from
	// a healthy, idle provider.
	successor := dialClient(t, url)
	if got := successor.registerMethods("brain.info"); contains(got, "brain.info") {
		t.Fatalf("expected the stale owner to keep the slot (that is the bug being fixed), ack was %v", got)
	}
}

// EvictConn is the deliberate release: the hub has decided this provider is
// dead (its liveness probe went unanswered) and frees the slot NOW, rather
// than waiting for a TCP timeout that may never come. Ownership must be gone
// by the time EvictConn returns, so a wake path can evict and then start a
// machine without racing the boot.
func TestEvictConnFreesTheProviderSlotSynchronously(t *testing.T) {
	url, srv := rpcServerWith(t)
	zombie := dialClient(t, url)
	if got := zombie.registerMethods("brain.info"); !contains(got, "brain.info") {
		t.Fatalf("first provider should own brain.info, got %v", got)
	}

	id, ok := srv.ProviderConnID("brain.info")
	if !ok {
		t.Fatal("ProviderConnID could not find the owner of brain.info")
	}
	if !srv.EvictConn(id) {
		t.Fatalf("EvictConn(%d) reported nothing to evict", id)
	}

	// No polling: the slot must be free the instant EvictConn returns.
	successor := dialClient(t, url)
	if got := successor.registerMethods("brain.info"); !contains(got, "brain.info") {
		t.Fatalf("successor could not claim brain.info after eviction, ack was %v", got)
	}
}

// Evicting a connection that does not exist (or has already gone) is a no-op,
// not a panic: the liveness poll and the wake path can both ask.
func TestEvictConnOnAnUnknownConnIsANoOp(t *testing.T) {
	_, srv := rpcServerWith(t)
	if srv.EvictConn(99999) {
		t.Fatal("EvictConn reported it evicted a connection that never existed")
	}
	if _, ok := srv.ProviderConnID("brain.info"); ok {
		t.Fatal("ProviderConnID found an owner for a method nobody registered")
	}
}

// The evicted socket really is closed, not merely deregistered — otherwise a
// half-alive far end could re-register into the slot it was just evicted from.
func TestEvictConnClosesTheSocket(t *testing.T) {
	url, srv := rpcServerWith(t)
	zombie := dialClient(t, url)
	zombie.registerMethods("brain.info")
	id, _ := srv.ProviderConnID("brain.info")
	srv.EvictConn(id)

	done := make(chan error, 1)
	go func() {
		_, _, err := zombie.ws.Read(t.Context())
		done <- err
	}()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("read on an evicted connection succeeded; the socket was not closed")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("read on an evicted connection did not return; the socket was not closed")
	}
}
