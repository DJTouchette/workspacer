package push

import (
	"crypto/elliptic"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"
	"unicode/utf8"

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
	m.notify = func(ev Event) {
		fired = append(fired, ev.SessionID+":"+ev.Title+":"+ev.Body)
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

	if got := endpoints(m.recipients(KindNeeds, 0)); len(got) != 2 {
		t.Fatalf("recipients before revocation = %v, want both devices", got)
	}

	// `workspacer token revoke` drops the tablet's token from the store.
	delete(live, "id-tablet")
	got := endpoints(m.recipients(KindNeeds, 0))
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
	if got := endpoints(reopened.recipients(KindNeeds, 0)); len(got) != 1 || got[0] != "https://push.example/phone" {
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
	if got := endpoints(m.recipients(KindNeeds, 0)); len(got) != 1 || got[0] != "https://push.example/legacy" {
		t.Fatalf("pre-identity subscription did not load: %v", got)
	}
	// A validator that rejects everything must not affect it — it has no
	// recorded credential to check.
	m.SetTokenValidator(func(string) bool { return false })
	if got := endpoints(m.recipients(KindNeeds, 0)); len(got) != 1 {
		t.Fatalf("recipients = %v, want the unattributed subscription kept", got)
	}
	// And with no validator at all (no token store wired), an attributed
	// subscription is delivered to as before.
	subscribeAs(t, m, "https://push.example/new", "id-new", "operator")
	m.SetTokenValidator(nil)
	if got := endpoints(m.recipients(KindNeeds, 0)); len(got) != 2 {
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
	// Three now, not two: the ended row is itself notifiable. The point of this
	// test is unchanged — the needs-you edge must be able to fire AGAIN after an
	// ended reset — so assert the two blocked notifications specifically rather
	// than a bare count that the new kind would satisfy by accident.
	var needs []string
	for _, f := range *fired {
		if strings.Contains(f, "needs you") {
			needs = append(needs, f)
		}
	}
	if len(needs) != 2 {
		t.Fatalf("expected 2 needs-you notifications across the ended reset, got %v", *fired)
	}
	if len(*fired) != 3 || !strings.Contains((*fired)[1], "Session ended") {
		t.Fatalf("expected the ended row to notify between them, got %v", *fired)
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

// ── finished / ended kinds ───────────────────────────────────────────────────

// fires captures (kind, body, ranFor) so the threshold logic can be asserted
// without a clock or a network.
func newKindManager(t *testing.T) (*Manager, *[]struct {
	Kind Kind
	Body string
	Ran  time.Duration
}) {
	t.Helper()
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	var fired []struct {
		Kind Kind
		Body string
		Ran  time.Duration
	}
	m.notify = func(ev Event) {
		fired = append(fired, struct {
			Kind Kind
			Body string
			Ran  time.Duration
		}{ev.Kind, ev.Body, ev.RanFor})
	}
	return m, &fired
}

// A run that goes idle is "finished"; the duration travels with it so each
// device can apply its own threshold.
func TestFinishedFiresOnWorkingToIdleWithDuration(t *testing.T) {
	m, fired := newKindManager(t)
	now := time.Unix(1_700_000_000, 0)
	m.now = func() time.Time { return now }

	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active"))
	now = now.Add(4 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "idle", "active"))

	if len(*fired) != 1 || (*fired)[0].Kind != KindFinished {
		t.Fatalf("expected one finished notification, got %+v", *fired)
	}
	if got := (*fired)[0].Ran; got != 4*time.Minute {
		t.Fatalf("run length: want 4m, got %v", got)
	}
	if got := (*fired)[0].Body; got != "Ran for 4m" {
		t.Fatalf("body: %q", got)
	}
}

// A dispatch is named on the lock screen the same way the /m fleet card titles
// it: by the task label it was dispatched with, and never by the directory it
// happens to be working in right now. Before this, the title led with `liveCwd`
// and ignored `label` entirely, so one worker announced itself as "workspacer"
// when it blocked and as "wks-viewport" when it landed, after entering a
// worktree in between.
func TestNotificationTitleIsTheDispatchLabelNotTheLiveDirectory(t *testing.T) {
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	var titles []string
	m.notify = func(ev Event) { titles = append(titles, ev.Title) }
	now := time.Unix(1_700_000_000, 0)
	m.now = func() time.Time { return now }

	labelled := func(ambient, liveCwd string) json.RawMessage {
		b, _ := json.Marshal(map[string]string{
			"sessionId": "s1", "cwd": "/x/workspacer", "liveCwd": liveCwd,
			"label": "workspacer: fix the viewport gap", "ambientState": ambient, "status": "active",
		})
		return b
	}
	m.onSnapshot(labelled("streaming", "/x/workspacer"))
	m.onSnapshot(labelled("waiting_approval", "/x/.worktrees/wks-viewport"))
	now = now.Add(3 * time.Minute)
	m.onSnapshot(labelled("idle", "/x/.worktrees/wks-viewport"))

	want := []string{
		"workspacer: fix the viewport gap needs you",
		"workspacer: fix the viewport gap landed",
	}
	if len(titles) != len(want) {
		t.Fatalf("titles: want %v, got %v", want, titles)
	}
	for i := range want {
		if titles[i] != want[i] {
			t.Errorf("title %d: want %q, got %q", i, want[i], titles[i])
		}
	}
}

// With no dispatch label the fallback is the directory the agent STARTED in —
// still not liveCwd, for the renaming reason above.
func TestUnlabelledNotificationFallsBackToTheStartDirectory(t *testing.T) {
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	var titles []string
	m.notify = func(ev Event) { titles = append(titles, ev.Title) }

	b, _ := json.Marshal(map[string]string{
		"sessionId": "s2", "cwd": "/x/preheat", "liveCwd": "/x/.worktrees/pre-1",
		"ambientState": "waiting_input", "status": "active",
	})
	m.onSnapshot(b)
	if len(titles) != 1 || titles[0] != "preheat needs you" {
		t.Fatalf("titles: %v", titles)
	}
}

// `background` means the turn ended but spawned work is still running — the run
// is not over, so no finish fires until it actually clears.
func TestBackgroundIsStillRunning(t *testing.T) {
	m, fired := newKindManager(t)
	now := time.Unix(1_700_000_000, 0)
	m.now = func() time.Time { return now }

	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active"))
	now = now.Add(30 * time.Second)
	m.onSnapshot(snap("s1", "/x/proj", "background", "active"))
	if len(*fired) != 0 {
		t.Fatalf("background must not read as finished, got %+v", *fired)
	}
	now = now.Add(90 * time.Second)
	m.onSnapshot(snap("s1", "/x/proj", "idle", "active"))
	if len(*fired) != 1 || (*fired)[0].Ran != 2*time.Minute {
		t.Fatalf("expected one finish spanning the whole run, got %+v", *fired)
	}
}

// Being blocked on you mid-run is part of the run: approving a tool and walking
// away must still earn the finish push, timed from when the work started.
func TestBlockedMidRunStillFinishes(t *testing.T) {
	m, fired := newKindManager(t)
	now := time.Unix(1_700_000_000, 0)
	m.now = func() time.Time { return now }

	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active"))
	now = now.Add(1 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "waiting_approval", "active")) // needs-you
	now = now.Add(2 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "idle", "active"))

	if len(*fired) != 2 || (*fired)[0].Kind != KindNeeds || (*fired)[1].Kind != KindFinished {
		t.Fatalf("expected needs then finished, got %+v", *fired)
	}
	if got := (*fired)[1].Ran; got != 3*time.Minute {
		t.Fatalf("run should span the approval wait: want 3m, got %v", got)
	}
}

// A session that was idle all along has no run behind it, so going idle again
// is not a finish.
func TestIdleWithoutARunNeverFinishes(t *testing.T) {
	m, fired := newKindManager(t)
	m.now = func() time.Time { return time.Unix(1_700_000_000, 0) }
	m.onSnapshot(snap("s1", "/x/proj", "idle", "active"))
	m.onSnapshot(snap("s1", "/x/proj", "idle", "active"))
	if len(*fired) != 0 {
		t.Fatalf("idle→idle must not notify, got %+v", *fired)
	}
}

// The ended rows the desktop republishes for long-dead sessions must not ring
// the phone every time a client reconnects — only a session we were tracking.
func TestEndedOnlyNotifiesForATrackedSession(t *testing.T) {
	m, fired := newKindManager(t)
	m.now = func() time.Time { return time.Unix(1_700_000_000, 0) }

	m.onSnapshot(snap("ghost", "/x/old", "idle", "ended")) // never seen before
	if len(*fired) != 0 {
		t.Fatalf("an untracked ended row must be silent, got %+v", *fired)
	}
	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active"))
	m.onSnapshot(snap("s1", "/x/proj", "idle", "ended"))
	if len(*fired) != 1 || (*fired)[0].Kind != KindEnded {
		t.Fatalf("expected one ended notification, got %+v", *fired)
	}
}

func boolp(b bool) *bool { return &b }
func intp(i int) *int    { return &i }

// Per-device filtering: the same edge reaches one phone and not another.
func TestRecipientsRespectPerDevicePrefs(t *testing.T) {
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	add := func(name string, p Prefs) {
		m.subs["https://push.example/"+name] = storedSub{
			Subscription: webpush.Subscription{Endpoint: "https://push.example/" + name},
			Prefs:        p,
		}
	}
	add("all", Prefs{})                                // unset = everything
	add("quiet", Prefs{Finished: boolp(false)})        // no finish pushes
	add("patient", Prefs{FinishedAfterSec: intp(300)}) // only runs over 5m
	add("eager", Prefs{FinishedAfterSec: intp(0)})     // any finish at all

	got := func(k Kind, ran time.Duration) []string {
		out := endpoints(m.recipients(k, ran))
		sort.Strings(out)
		return out
	}

	// A 90s run: over the 60s default, under "patient"'s 5m.
	if e := got(KindFinished, 90*time.Second); len(e) != 2 ||
		e[0] != "https://push.example/all" || e[1] != "https://push.example/eager" {
		t.Fatalf("90s finish went to %v", e)
	}
	// A 10m run reaches everyone who wants finishes.
	if e := got(KindFinished, 10*time.Minute); len(e) != 3 {
		t.Fatalf("10m finish went to %v", e)
	}
	// A 5s run is under every threshold except the eager one's zero.
	if e := got(KindFinished, 5*time.Second); len(e) != 1 || e[0] != "https://push.example/eager" {
		t.Fatalf("5s finish went to %v", e)
	}
	// needs-you is untouched by any of the finish settings.
	if e := got(KindNeeds, 0); len(e) != 4 {
		t.Fatalf("needs-you must reach every device, got %v", e)
	}
}

// A subscription stored before prefs existed decodes to all-unset, which must
// read as "send me everything" — not as a silently muted device.
func TestLegacySubscriptionKeepsReceiving(t *testing.T) {
	var p Prefs
	if err := json.Unmarshal([]byte(`{}`), &p); err != nil {
		t.Fatal(err)
	}
	for _, k := range []Kind{KindNeeds, KindFinished, KindEnded} {
		if !p.wants(k) {
			t.Fatalf("unset prefs must want %q", k)
		}
	}
	if p.finishedAfter() != DefaultFinishedAfter {
		t.Fatalf("unset threshold: want default, got %v", p.finishedAfter())
	}
}

// The wire contract with the phone: mobile.html sends prefs alongside the
// browser's PushSubscription on the SAME push.subscribe call. If this shape
// drifts, prefs silently revert to defaults and every toggle stops working
// with nothing to show for it.
func TestSubscribeStoresPrefsFromTheWire(t *testing.T) {
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	// Exactly what the client sends: PushSubscription.toJSON() + prefs.
	params := []byte(`{
	  "endpoint": "https://push.example/phone",
	  "keys": { "p256dh": "k", "auth": "a" },
	  "prefs": { "needs": true, "finished": false, "ended": true, "finishedAfterSec": 300 }
	}`)
	if _, err := m.RPCSubscribeAs(Subscriber{}, params); err != nil {
		t.Fatalf("subscribe: %v", err)
	}
	got := m.subs["https://push.example/phone"].Prefs
	if got.Finished == nil || *got.Finished {
		t.Fatalf("finished should be stored as false, got %+v", got.Finished)
	}
	if got.FinishedAfterSec == nil || *got.FinishedAfterSec != 300 {
		t.Fatalf("threshold not stored: %+v", got.FinishedAfterSec)
	}
	// And it actually filters: this device wants no finishes at all.
	if e := endpoints(m.recipients(KindFinished, time.Hour)); len(e) != 0 {
		t.Fatalf("a device with finished:false still got one: %v", e)
	}
	if e := endpoints(m.recipients(KindNeeds, 0)); len(e) != 1 {
		t.Fatalf("needs-you must still reach it, got %v", e)
	}
	// Survives a reload — prefs are part of the persisted record.
	reopened, err := New(m.dir)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	if e := endpoints(reopened.recipients(KindFinished, time.Hour)); len(e) != 0 {
		t.Fatalf("prefs lost across reload: %v", e)
	}
}

// The test push is the affordance that answers "is push reaching me at all",
// so it must ignore the per-kind mutes — a test a setting could silence cannot
// distinguish "muted" from "broken", which is the whole reason it exists.
func TestTestPushIgnoresMutedKinds(t *testing.T) {
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	m.subs["https://push.example/silent"] = storedSub{
		Subscription: webpush.Subscription{Endpoint: "https://push.example/silent"},
		Prefs:        Prefs{Needs: boolp(false), Finished: boolp(false), Ended: boolp(false)},
	}
	var sent []string
	m.send = func(sub webpush.Subscription, _ []byte) (int, error) {
		sent = append(sent, sub.Endpoint)
		return 201, nil
	}

	res, err := m.RPCTest(nil)
	if err != nil {
		t.Fatalf("RPCTest: %v", err)
	}
	if len(sent) != 1 || sent[0] != "https://push.example/silent" {
		t.Fatalf("expected one send to the muted device, got %v", sent)
	}
	// It reports what actually HAPPENED, not what it attempted — the first
	// version returned len(recipients) before any send completed, so it said
	// "sent to 4 devices" while four dead endpoints were about to 410.
	got := res.(map[string]any)
	if got["delivered"] != 1 || got["gone"] != 0 || got["failed"] != 0 {
		t.Fatalf("counts should reflect the real outcome: %v", got)
	}
	if e := endpoints(m.recipients(KindTest, 0)); len(e) != 1 {
		t.Fatalf("test push must reach a fully-muted device, got %v", e)
	}
	// And the mutes it ignores are still honoured for real notifications.
	if e := endpoints(m.recipients(KindNeeds, 0)); len(e) != 0 {
		t.Fatalf("needs-you must still be muted for that device, got %v", e)
	}
}

// The reported bug: "Sent to 4 devices" while nothing arrived. The count was
// len(recipients), taken before any delivery completed, so stale endpoints read
// as success. This is that exact fleet — one live phone, two dead endpoints and
// one refusal — and the numbers must tell them apart.
func TestTestPushReportsOutcomesNotAttempts(t *testing.T) {
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	for _, e := range []string{"live", "gone1", "gone2", "refused"} {
		m.subs["https://push.example/"+e] = storedSub{
			Subscription: webpush.Subscription{Endpoint: "https://push.example/" + e},
		}
	}
	m.send = func(sub webpush.Subscription, _ []byte) (int, error) {
		switch {
		case strings.HasSuffix(sub.Endpoint, "/live"):
			return 201, nil
		case strings.Contains(sub.Endpoint, "/gone"):
			return 410, nil
		default:
			return 400, nil
		}
	}

	res, err := m.RPCTest(nil)
	if err != nil {
		t.Fatalf("RPCTest: %v", err)
	}
	got := res.(map[string]any)
	if got["devices"] != 4 {
		t.Fatalf("devices should be everything it tried: %v", got)
	}
	if got["delivered"] != 1 {
		t.Fatalf("exactly one endpoint accepted it; delivered=%v (if this equals devices, the count is of ATTEMPTS again)", got["delivered"])
	}
	if got["gone"] != 2 {
		t.Fatalf("two endpoints were 410; gone=%v", got["gone"])
	}
	if got["failed"] != 1 {
		t.Fatalf("one endpoint refused; failed=%v", got["failed"])
	}
}

// Apple refuses a VAPID JWT whose `sub` is not a mailto: with a real domain or
// an https: URL. It was "mailto:workspacer@localhost" — which FCM accepts, so
// Chrome and Android worked while web.push.apple.com returned 403 on every
// notification, forever. The one platform that refused it was the one the
// feature exists for.
func TestVAPIDSubjectIsAcceptableToApple(t *testing.T) {
	u, err := url.Parse(vapidSubject)
	if err != nil {
		t.Fatalf("vapidSubject %q does not parse: %v", vapidSubject, err)
	}
	switch u.Scheme {
	case "https":
		if u.Host == "" || !strings.Contains(u.Host, ".") {
			t.Fatalf("https subject needs a real host, got %q", vapidSubject)
		}
	case "mailto":
		// url.Parse puts the address in Opaque for mailto:.
		at := strings.LastIndex(u.Opaque, "@")
		if at < 0 || !strings.Contains(u.Opaque[at+1:], ".") {
			t.Fatalf("mailto subject needs a real domain (this is exactly what @localhost failed), got %q", vapidSubject)
		}
	default:
		t.Fatalf("VAPID sub must be mailto: or https:, got %q", vapidSubject)
	}
}

// ── preview + checkpoints ────────────────────────────────────────────────────

func snapRich(sessionID, cwd, ambient string, extra map[string]any) json.RawMessage {
	m := map[string]any{"sessionId": sessionID, "cwd": cwd, "ambientState": ambient, "status": "active"}
	for k, v := range extra {
		m[k] = v
	}
	b, _ := json.Marshal(m)
	return b
}

// "Approve a tool use" is useless next to "Bash rm -rf build/". The content was
// on the snapshot the whole time; onSnapshot decoded five scalars and dropped it.
func TestApprovalPreviewNamesTheCommand(t *testing.T) {
	m, err := New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	var got Event
	m.notify = func(ev Event) { got = ev }
	m.onSnapshot(snapRich("s1", "/x/proj", "waiting_approval", map[string]any{
		"pendingApproval": map[string]any{
			"toolName":  "Bash",
			"toolInput": map[string]any{"command": "rm -rf build/"},
		},
	}))
	if got.Kind != KindNeeds {
		t.Fatalf("kind: %v", got.Kind)
	}
	if got.Body != "Approve a tool use" {
		t.Fatalf("body should stay content-free: %q", got.Body)
	}
	if got.Detail != "Bash rm -rf build/" {
		t.Fatalf("detail: %q", got.Detail)
	}
}

func TestQuestionPreviewCarriesTheQuestion(t *testing.T) {
	m, _ := New(t.TempDir())
	var got Event
	m.notify = func(ev Event) { got = ev }
	m.onSnapshot(snapRich("s1", "/x/proj", "waiting_input", map[string]any{
		"pendingQuestions": map[string]any{
			"questions": []map[string]any{{"question": "Which database should I target?"}},
		},
	}))
	if got.Detail != "Which database should I target?" {
		t.Fatalf("detail: %q", got.Detail)
	}
}

// The preview is per-device, so it has to be applied at DELIVERY. A device that
// turned it off must get the body and never the agent's words.
func TestPreviewIsPerDeviceAtDelivery(t *testing.T) {
	m, _ := New(t.TempDir())
	no := false
	m.subs["https://push.example/open"] = storedSub{
		Subscription: webpush.Subscription{Endpoint: "https://push.example/open"},
	}
	m.subs["https://push.example/private"] = storedSub{
		Subscription: webpush.Subscription{Endpoint: "https://push.example/private"},
		Prefs:        Prefs{Preview: &no},
	}
	// sendAll fans out over one goroutine per subscription (push.go), so this
	// hook runs on several at once while the test goroutine reads what they
	// wrote. Both halves take the same lock: the map was previously
	// unsynchronized and raced on every run — it passed on a fast dev machine
	// and failed on the shared CI runner, which is the worst way for a test to
	// be wrong, because the machine that would show you is the one you do not
	// watch.
	var mu sync.Mutex
	bodies := map[string]string{}
	both := make(chan struct{})
	m.send = func(sub webpush.Subscription, payload []byte) (int, error) {
		var p map[string]string
		_ = json.Unmarshal(payload, &p)
		mu.Lock()
		defer mu.Unlock()
		bodies[sub.Endpoint] = p["body"]
		// Signalled on the exact count rather than polled for: a sleep-loop has
		// to sample the map to know when to stop, which is the read that raced.
		// Closing here means the wait below observes a map that is finished
		// being written, with no timing assumption at all.
		if len(bodies) == 2 {
			close(both)
		}
		return 201, nil
	}
	m.sendAll(Event{Kind: KindNeeds, Title: "proj needs you", Body: "Approve a tool use",
		Detail: "Bash rm -rf build/", SessionID: "s1"})

	select {
	case <-both:
	case <-time.After(5 * time.Second):
		// Generous, because it is now only a deadlock backstop rather than the
		// thing the assertions depend on: reaching it means a send never
		// happened, which is a real failure and says so.
		mu.Lock()
		got := len(bodies)
		mu.Unlock()
		t.Fatalf("timed out: only %d of 2 devices were sent to", got)
	}

	mu.Lock()
	defer mu.Unlock()
	if got := bodies["https://push.example/open"]; got != "Approve a tool use — Bash rm -rf build/" {
		t.Fatalf("preview device body: %q", got)
	}
	if got := bodies["https://push.example/private"]; got != "Approve a tool use" {
		t.Fatalf("a device with preview OFF must not receive the agent's words, got %q", got)
	}
}

func TestClipCutsOnRuneBoundaries(t *testing.T) {
	long := strings.Repeat("é", MaxDetailChars+40)
	out := clip(long)
	if !strings.HasSuffix(out, "…") {
		t.Fatalf("expected an ellipsis, got %q", out[len(out)-8:])
	}
	if r := []rune(out); len(r) != MaxDetailChars+1 {
		t.Fatalf("clipped to %d runes, want %d", len(r), MaxDetailChars+1)
	}
	if !utf8.ValidString(out) {
		t.Fatal("clip produced invalid UTF-8 — a rune was cut in half")
	}
}

// Checkpoints fire while a run is STILL going, once per mark, and are opt-in.
func TestCheckpointsFireOncePerMarkAndAreOptIn(t *testing.T) {
	m, _ := New(t.TempDir())
	now := time.Unix(1_700_000_000, 0)
	m.now = func() time.Time { return now }
	var kinds []Kind
	m.notify = func(ev Event) { kinds = append(kinds, ev.Kind) }

	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active"))
	now = now.Add(11 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active")) // crosses 10m
	now = now.Add(1 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active")) // still past 10m, must not repeat
	if len(kinds) != 1 || kinds[0] != KindCheckpoint {
		t.Fatalf("expected exactly one 10m checkpoint, got %v", kinds)
	}
	now = now.Add(20 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active")) // crosses 30m
	if len(kinds) != 2 {
		t.Fatalf("expected the 30m checkpoint too, got %v", kinds)
	}

	// A new run resets the marks.
	now = now.Add(1 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "idle", "active")) // finishes
	now = now.Add(1 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active")) // new run
	now = now.Add(11 * time.Minute)
	m.onSnapshot(snap("s1", "/x/proj", "streaming", "active"))
	last := kinds[len(kinds)-1]
	if last != KindCheckpoint {
		t.Fatalf("a new run should checkpoint again, got %v", kinds)
	}

	// Opt-in: unset prefs must NOT receive them, unlike every other kind.
	var p Prefs
	if p.wants(KindCheckpoint) {
		t.Fatal("checkpoints must be off by default — they fire on a timer, not on an event")
	}
	yes := true
	if !(Prefs{Checkpoints: &yes}).wants(KindCheckpoint) {
		t.Fatal("enabling checkpoints must turn them on")
	}
}
