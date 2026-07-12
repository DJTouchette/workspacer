package push

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// newTestManager builds a Manager against a temp dir with the network send
// stubbed out, capturing notifications instead.
func newTestManager(t *testing.T) (*Manager, *[]string) {
	t.Helper()
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	var fired []string
	m.notify = func(title, body, sessionID string) {
		fired = append(fired, sessionID+":"+title+":"+body)
	}
	return m, &fired
}

func snap(sessionID, cwd, ambient, status string) json.RawMessage {
	b, _ := json.Marshal(map[string]string{
		"sessionId": sessionID, "cwd": cwd, "ambientState": ambient, "status": status,
	})
	return b
}

func TestNewGeneratesAndPersistsVAPID(t *testing.T) {
	dir := t.TempDir()
	m1, err := New(dir)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	if m1.PublicKey() == "" {
		t.Fatal("expected a generated VAPID public key")
	}
	if _, err := os.Stat(filepath.Join(dir, "vapid.json")); err != nil {
		t.Fatalf("vapid.json not persisted: %v", err)
	}
	// A second Manager over the same dir must reuse the SAME key (so a phone that
	// already subscribed against the old key keeps working across restarts).
	m2, err := New(dir)
	if err != nil {
		t.Fatalf("New (reopen): %v", err)
	}
	if m2.PublicKey() != m1.PublicKey() {
		t.Fatal("VAPID public key changed across restarts")
	}
}

func TestSubscribePersistsAndReloads(t *testing.T) {
	dir := t.TempDir()
	m, err := New(dir)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	sub := json.RawMessage(`{"endpoint":"https://push.example/abc","keys":{"p256dh":"pk","auth":"au"}}`)
	if _, err := m.RPCSubscribe(sub); err != nil {
		t.Fatalf("RPCSubscribe: %v", err)
	}
	// Missing keys must be rejected.
	if _, err := m.RPCSubscribe(json.RawMessage(`{"endpoint":"https://push.example/x"}`)); err == nil {
		t.Fatal("expected error for subscription missing keys")
	}
	// Reload from disk sees exactly the one valid subscription.
	m2, err := New(dir)
	if err != nil {
		t.Fatalf("New (reopen): %v", err)
	}
	if len(m2.subs) != 1 {
		t.Fatalf("expected 1 persisted subscription, got %d", len(m2.subs))
	}
	// Unsubscribe (with the matching auth) removes it and persists the removal.
	if _, err := m.RPCUnsubscribe(json.RawMessage(`{"endpoint":"https://push.example/abc","auth":"au"}`)); err != nil {
		t.Fatalf("RPCUnsubscribe: %v", err)
	}
	m3, _ := New(dir)
	if len(m3.subs) != 0 {
		t.Fatalf("expected 0 subscriptions after unsubscribe, got %d", len(m3.subs))
	}
}

// TestUnsubscribeRequiresProofOfPossession covers the ownership gate on
// RPCUnsubscribe: the bus RPC layer has no per-caller identity, so a legitimate
// owner proves possession by presenting the subscription's `keys.auth` secret
// (which its own browser holds). A caller with only the opaque endpoint cannot.
func TestUnsubscribeRequiresProofOfPossession(t *testing.T) {
	dir := t.TempDir()
	m, err := New(dir)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	sub := json.RawMessage(`{"endpoint":"https://push.example/abc","keys":{"p256dh":"pk","auth":"secret-auth"}}`)
	if _, err := m.RPCSubscribe(sub); err != nil {
		t.Fatalf("RPCSubscribe: %v", err)
	}

	// (b) Wrong auth is rejected and the subscription SURVIVES.
	if _, err := m.RPCUnsubscribe(json.RawMessage(`{"endpoint":"https://push.example/abc","auth":"wrong"}`)); err == nil {
		t.Fatal("expected unsubscribe with wrong auth to be rejected")
	}
	// Missing auth is likewise rejected.
	if _, err := m.RPCUnsubscribe(json.RawMessage(`{"endpoint":"https://push.example/abc"}`)); err == nil {
		t.Fatal("expected unsubscribe with missing auth to be rejected")
	}
	m.mu.Lock()
	survived := len(m.subs)
	m.mu.Unlock()
	if survived != 1 {
		t.Fatalf("subscription should survive a bad-auth unsubscribe, got %d subs", survived)
	}

	// (c) Unknown endpoint is a no-op success (idempotent), even without auth.
	if _, err := m.RPCUnsubscribe(json.RawMessage(`{"endpoint":"https://push.example/unknown"}`)); err != nil {
		t.Fatalf("unsubscribe of an unknown endpoint should be a no-op success, got %v", err)
	}

	// (a) Correct auth removes the subscription and persists the removal.
	if _, err := m.RPCUnsubscribe(json.RawMessage(`{"endpoint":"https://push.example/abc","auth":"secret-auth"}`)); err != nil {
		t.Fatalf("unsubscribe with correct auth: %v", err)
	}
	reopened, _ := New(dir)
	if len(reopened.subs) != 0 {
		t.Fatalf("expected 0 subscriptions after authorized unsubscribe, got %d", len(reopened.subs))
	}
}

func TestNotifiesOnlyOnBlockedEdge(t *testing.T) {
	m, fired := newTestManager(t)

	// streaming → not blocked: no push.
	m.onSnapshot(snap("s1", "/home/me/proj", "streaming", "active"))
	if len(*fired) != 0 {
		t.Fatalf("streaming should not notify, got %v", *fired)
	}
	// → waiting_approval: the un-blocked→blocked edge fires once.
	m.onSnapshot(snap("s1", "/home/me/proj", "waiting_approval", "active"))
	if len(*fired) != 1 {
		t.Fatalf("expected 1 notification on the blocked edge, got %v", *fired)
	}
	if got := (*fired)[0]; got != "s1:proj needs you:Approve a tool use" {
		t.Fatalf("unexpected notification payload: %q", got)
	}
	// A repeat waiting_approval snapshot must NOT re-fire (still blocked).
	m.onSnapshot(snap("s1", "/home/me/proj", "waiting_approval", "active"))
	if len(*fired) != 1 {
		t.Fatalf("repeat blocked snapshot should not re-notify, got %v", *fired)
	}
	// User answers → streaming, then blocks again → fires a second time.
	m.onSnapshot(snap("s1", "/home/me/proj", "streaming", "active"))
	m.onSnapshot(snap("s1", "/home/me/proj", "waiting_input", "active"))
	if len(*fired) != 2 {
		t.Fatalf("expected a second notification after re-entering blocked, got %v", *fired)
	}
	if got := (*fired)[1]; got != "s1:proj needs you:Answer a question" {
		t.Fatalf("unexpected second payload: %q", got)
	}
}

func TestEndedSessionResetsSoItCanRefire(t *testing.T) {
	m, fired := newTestManager(t)
	m.onSnapshot(snap("s2", "/x/y", "waiting_approval", "active"))
	m.onSnapshot(snap("s2", "/x/y", "waiting_approval", "ended"))  // ended clears state
	m.onSnapshot(snap("s2", "/x/y", "waiting_approval", "active")) // fresh edge → fires again
	if len(*fired) != 2 {
		t.Fatalf("expected 2 notifications across the ended reset, got %v", *fired)
	}
}

func TestMissingAmbientStateNeverFires(t *testing.T) {
	m, fired := newTestManager(t)
	// A brain/claudemon-backed snapshot has no ambientState — must never notify.
	m.onSnapshot(json.RawMessage(`{"session_id":"s3","cwd":"/a/b"}`))
	if len(*fired) != 0 {
		t.Fatalf("snapshot without ambientState should not notify, got %v", *fired)
	}
}
