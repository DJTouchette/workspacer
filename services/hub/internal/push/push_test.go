package push

import (
	"crypto/elliptic"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"testing"
	"time"

	webpush "github.com/SherClockHolmes/webpush-go"
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

// subscribeAs registers a device against a token identity, the way the bus
// wiring does for a real /m subscribe.
func subscribeAs(t *testing.T, m *Manager, endpoint, tokenID, scope string) {
	t.Helper()
	params, _ := json.Marshal(map[string]any{
		"endpoint": endpoint,
		"keys":     map[string]string{"p256dh": "pk", "auth": "au-" + endpoint},
	})
	if _, err := m.RPCSubscribeAs(Subscriber{TokenID: tokenID, Scope: scope}, params); err != nil {
		t.Fatalf("RPCSubscribeAs(%s): %v", endpoint, err)
	}
}

func endpoints(subs []webpush.Subscription) []string {
	out := make([]string, 0, len(subs))
	for _, s := range subs {
		out = append(out, s.Endpoint)
	}
	sort.Strings(out)
	return out
}

// The finding: a push subscription outlives the connection that created it, so
// revoking a device's token cut its bus access while leaving it receiving
// "<repo> needs you" forever — the only remedy was stopping the hub and editing
// push-subscriptions.json by hand. Recording the subscriber's token identity
// makes revocation cut push the same way it cuts the bus.
func TestRevokedTokenStopsReceivingPushes(t *testing.T) {
	dir := t.TempDir()
	m, err := New(dir)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	subscribeAs(t, m, "https://push.example/phone", "id-phone", "triage")
	subscribeAs(t, m, "https://push.example/tablet", "id-tablet", "triage")

	live := map[string]bool{"id-phone": true, "id-tablet": true}
	m.SetTokenValidator(func(id string) bool { return live[id] })

	if got := endpoints(m.recipients()); len(got) != 2 {
		t.Fatalf("recipients before revocation = %v, want both devices", got)
	}

	// `workspacer token revoke` drops the tablet's token from the store.
	delete(live, "id-tablet")
	got := endpoints(m.recipients())
	if len(got) != 1 || got[0] != "https://push.example/phone" {
		t.Fatalf("recipients after revoking the tablet = %v, want only the phone", got)
	}

	// Skipped, not deleted: the validator is backed by a file that fails closed
	// on a transient read error, so a momentary "no" must not unsubscribe every
	// device permanently.
	m.mu.Lock()
	stored := len(m.subs)
	m.mu.Unlock()
	if stored != 2 {
		t.Fatalf("stored subscriptions = %d, want both kept (revocation filters, it does not delete)", stored)
	}

	// The identity survives a restart, or revocation would only hold until the
	// hub next started.
	reopened, err := New(dir)
	if err != nil {
		t.Fatalf("New (reopen): %v", err)
	}
	reopened.SetTokenValidator(func(id string) bool { return live[id] })
	if got := endpoints(reopened.recipients()); len(got) != 1 || got[0] != "https://push.example/phone" {
		t.Fatalf("recipients after restart = %v, want only the phone", got)
	}
}

// Two cases that must keep ringing: a hub with no token configured (the
// loopback default has no revocation authority to consult) and a subscription
// stored before identity was recorded. Failing closed on either would silently
// switch off notifications for existing installs.
func TestUnattributedSubscriptionsStillReceive(t *testing.T) {
	dir := t.TempDir()
	old := `[{"endpoint":"https://push.example/legacy","keys":{"p256dh":"pk","auth":"au"}}]`
	if err := os.WriteFile(filepath.Join(dir, "push-subscriptions.json"), []byte(old), 0o600); err != nil {
		t.Fatal(err)
	}
	m, err := New(dir)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	if got := endpoints(m.recipients()); len(got) != 1 || got[0] != "https://push.example/legacy" {
		t.Fatalf("pre-identity subscription did not load: %v", got)
	}
	// A validator that rejects everything must not affect it — it has no
	// recorded credential to check.
	m.SetTokenValidator(func(string) bool { return false })
	if got := endpoints(m.recipients()); len(got) != 1 {
		t.Fatalf("recipients = %v, want the unattributed subscription kept", got)
	}
	// And with no validator at all (no token store wired), an attributed
	// subscription is delivered to as before.
	subscribeAs(t, m, "https://push.example/new", "id-new", "operator")
	m.SetTokenValidator(nil)
	if got := endpoints(m.recipients()); len(got) != 2 {
		t.Fatalf("recipients with no validator = %v, want both", got)
	}
}

// push.list / push.revoke are the operator's remedy: see which devices are
// registered and cut one without holding its `keys.auth` secret — the
// proof-of-possession gate on push.unsubscribe is exactly the wrong shape when
// the device is the thing being revoked.
func TestListAndRevokeAreTheOperatorRemedy(t *testing.T) {
	dir := t.TempDir()
	m, err := New(dir)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	subscribeAs(t, m, "https://push.example/phone", "id-phone", "triage")
	subscribeAs(t, m, "https://push.example/tablet", "id-gone", "triage")
	subscribeAs(t, m, "https://push.example/tablet2", "id-gone", "triage")
	m.SetTokenValidator(func(id string) bool { return id == "id-phone" })

	listed, err := m.RPCList(nil)
	if err != nil {
		t.Fatalf("RPCList: %v", err)
	}
	rows := listed.(map[string]any)["subscriptions"].([]map[string]any)
	if len(rows) != 3 {
		t.Fatalf("listed %d subscriptions, want 3", len(rows))
	}
	revokedSeen := 0
	for _, r := range rows {
		if r["revoked"].(bool) {
			revokedSeen++
		}
		if r["tokenId"] == "" {
			t.Errorf("row %v has no recorded identity", r)
		}
	}
	if revokedSeen != 2 {
		t.Fatalf("%d rows flagged revoked, want the 2 registered by the dead token", revokedSeen)
	}

	// Revoking by token identity clears every device that credential registered,
	// in one call and without any of their auth secrets.
	res, err := m.RPCRevoke(json.RawMessage(`{"tokenId":"` + "id-gone" + `"}`))
	if err != nil {
		t.Fatalf("RPCRevoke by tokenId: %v", err)
	}
	if n := res.(map[string]any)["removed"].(int); n != 2 {
		t.Fatalf("removed = %d, want 2", n)
	}

	// Revoking by endpoint takes the remaining one, and the removal persists.
	if _, err := m.RPCRevoke(json.RawMessage(`{"endpoint":"https://push.example/phone"}`)); err != nil {
		t.Fatalf("RPCRevoke by endpoint: %v", err)
	}
	reopened, _ := New(dir)
	if len(reopened.subs) != 0 {
		t.Fatalf("expected 0 subscriptions after revocation, got %d", len(reopened.subs))
	}

	// A revoke that matches nothing is a no-op success; a revoke naming neither
	// selector is an error, so a typo can't quietly clear the whole store.
	if _, err := m.RPCRevoke(json.RawMessage(`{"endpoint":"https://push.example/unknown"}`)); err != nil {
		t.Fatalf("revoking an unknown endpoint should be a no-op success, got %v", err)
	}
	if _, err := m.RPCRevoke(json.RawMessage(`{}`)); err == nil {
		t.Fatal("expected an error when push.revoke names neither endpoint nor tokenId")
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

// A push send must never be able to block forever. A push service reachable
// through a captive portal or a stalling VPN completes the TCP handshake and
// then never answers; with no deadline the goroutine holds its socket for the
// life of the process, one more per stored subscription on every transition.
func TestPushClientHasATimeout(t *testing.T) {
	if pushClient.Timeout <= 0 {
		t.Fatal("pushClient.Timeout must be set — an unbounded send leaks a goroutine and a socket")
	}
	if pushClient.Timeout != pushTimeout {
		t.Errorf("pushClient.Timeout = %s, want %s", pushClient.Timeout, pushTimeout)
	}
}

func TestSendOneGivesUpOnAServerThatNeverResponds(t *testing.T) {
	// Accepts the connection, then never writes a byte — the exact shape of the
	// hang this bounds.
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			defer c.Close()
		}
	}()

	vapidPriv, vapidPub, err := webpush.GenerateVAPIDKeys()
	if err != nil {
		t.Fatal(err)
	}
	m := &Manager{vapidPub: vapidPub, vapidKey: vapidPriv}

	// A real P-256 point and a 16-byte auth secret, so encryption succeeds and
	// the call actually reaches the network — otherwise it would fail early and
	// the test would prove nothing about the deadline.
	priv, x, y, err := elliptic.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	_ = priv
	auth := make([]byte, 16)
	if _, err := rand.Read(auth); err != nil {
		t.Fatal(err)
	}
	sub := webpush.Subscription{
		Endpoint: "http://" + ln.Addr().String() + "/push",
		Keys: webpush.Keys{
			Auth:   base64.RawURLEncoding.EncodeToString(auth),
			P256dh: base64.RawURLEncoding.EncodeToString(elliptic.Marshal(elliptic.P256(), x, y)),
		},
	}

	// Shorten the deadline rather than sitting out the real one; the value
	// itself is pinned by TestPushClientHasATimeout.
	restore := pushClient
	pushClient = &http.Client{Timeout: 300 * time.Millisecond}
	defer func() { pushClient = restore }()

	done := make(chan struct{})
	go func() {
		m.sendOne(sub, []byte(`{"title":"x"}`))
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("sendOne did not return — the send is unbounded")
	}
}
