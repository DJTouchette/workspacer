// Package push turns agent "needs you" transitions into Web Push notifications,
// so a phone with the /m PWA installed gets a lock-screen alert when one of its
// agents is waiting on an approval or a question — even with the app closed.
//
// It owns a VAPID keypair and the set of browser push subscriptions (both
// persisted under the hub state dir), and watches the bus for `agent.snapshot`
// events. When a session's ambientState crosses into `waiting_approval` /
// `waiting_input` (the same "blocked" edge the desktop notifies on), it sends a
// push to every subscription. A subscription the push service reports as gone
// (404/410) is pruned.
//
// A note on the ceiling: this does NOT keep a socket alive in the background —
// mobile OSes forbid that. It wakes the service worker on demand to show a
// notification, which is the reliable web mechanism for background awareness.
package push

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"log"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"sync"
	"time"

	webpush "github.com/SherClockHolmes/webpush-go"
	"github.com/djtouchette/workspacer-hub/internal/broker"
)

// VAPID `sub` claim: who to contact about this push sender.
//
// Must be a mailto: with a REAL domain or an https: URL. This was
// "mailto:workspacer@localhost", which FCM accepts and Apple does not —
// localhost is not a valid email domain, so web.push.apple.com rejected the
// JWT with 403 on every single notification. Chrome and Android worked
// throughout, which is why it survived: the one platform that refused it was
// the one platform the feature was built for.
//
// An https: URL avoids inventing an address nobody reads. Changing the subject
// does NOT invalidate existing subscriptions — it is a claim inside the signed
// JWT, not part of the keypair the browser subscribed against.
const vapidSubject = "https://github.com/DJTouchette/workspacer"

type vapidKeys struct {
	PublicKey  string `json:"publicKey"`
	PrivateKey string `json:"privateKey"`
}

// Subscriber is the identity behind a push registration: which credential the
// device presented when it subscribed. Recorded so revoking that credential
// also cuts the notifications — before this, a revoked phone kept getting
// "<repo> needs you" forever and the only remedy was stopping the hub and
// editing push-subscriptions.json by hand.
type Subscriber struct {
	// TokenID is an opaque, stable fingerprint of the bearer token the
	// subscribing connection presented — never the token itself, so the
	// subscription store doesn't become a second place a credential can leak
	// from. The bus produces it (bus.TokenFingerprint), and whoever wires
	// SetTokenValidator fingerprints the live token set with that same function;
	// this package only ever compares the strings. Empty when the hub runs
	// without a token (the loopback default) or for registrations stored before
	// identity was recorded.
	TokenID string
	// Scope is the tier that token authenticated as ("triage", "operator", …),
	// kept for the operator listing so a device can be recognized.
	Scope string
}

// storedSub is one persisted registration: the browser's PushSubscription plus
// who registered it. The Subscription is embedded so the on-disk shape stays
// the array of {endpoint, keys} it always was — a file written before
// subscriber identity existed loads with an empty [Subscriber].
type storedSub struct {
	webpush.Subscription
	TokenID string    `json:"tokenId,omitempty"`
	Scope   string    `json:"scope,omitempty"`
	Created time.Time `json:"created,omitempty"`
	Prefs   Prefs     `json:"prefs,omitempty"`
}

// Kind is what happened. Each is separately switchable per device, because the
// right answer differs by person and by phone: "needs you" is the one nobody
// wants to miss, "finished" is the one that turns into noise fastest.
type Kind string

const (
	KindNeeds    Kind = "needs"    // an agent is blocked on you
	KindFinished Kind = "finished" // a long run went idle
	KindEnded    Kind = "ended"    // the session exited
	// KindTest is a notification you asked for by hand. It deliberately has no
	// entry in Prefs.wants, whose default is "yes": a test that a muted setting
	// could silence would be useless for answering "is push reaching me at all".
	KindTest Kind = "test"
)

// DefaultFinishedAfter is how long a run must have been working before going
// idle earns a push. Every turn ends, so an unconditional "finished" fires on
// the twenty-second exchange you are already watching; the threshold is what
// makes it mean "the thing you walked away from is done".
const DefaultFinishedAfter = 60 * time.Second

// Prefs is one device's notification settings. Every field is a POINTER so that
// absent means "unset, use the default" rather than the zero value — a
// subscription persisted before prefs existed, or one from a client that has
// not been taught about a newly added Kind, must keep receiving rather than
// silently go quiet.
type Prefs struct {
	Needs            *bool `json:"needs,omitempty"`
	Finished         *bool `json:"finished,omitempty"`
	Ended            *bool `json:"ended,omitempty"`
	FinishedAfterSec *int  `json:"finishedAfterSec,omitempty"`
}

// wants reports whether this device should receive `k`. Unset = yes.
func (p Prefs) wants(k Kind) bool {
	switch k {
	case KindNeeds:
		return p.Needs == nil || *p.Needs
	case KindFinished:
		return p.Finished == nil || *p.Finished
	case KindEnded:
		return p.Ended == nil || *p.Ended
	}
	return true
}

// finishedAfter is this device's threshold; unset or negative uses the default,
// and zero means "as soon as it goes idle, however short the run".
func (p Prefs) finishedAfter() time.Duration {
	if p.FinishedAfterSec == nil || *p.FinishedAfterSec < 0 {
		return DefaultFinishedAfter
	}
	return time.Duration(*p.FinishedAfterSec) * time.Second
}

// Manager is safe for concurrent use. The snapshot watcher runs on one
// goroutine (so `states` needs no lock); `subs` is guarded by `mu`.
type Manager struct {
	dir      string
	vapidPub string
	vapidKey string

	mu   sync.Mutex
	subs map[string]storedSub // keyed by endpoint
	// tokenValid reports whether a recorded token identity is still live. Nil
	// (the default, and what a hub with no token store has) means "no revocation
	// authority to consult" — every subscription is delivered to, which is the
	// behaviour that existed before revocation was wired.
	tokenValid func(tokenID string) bool

	states map[string]sessionState // sessionId -> last seen (watcher goroutine only)

	// notify is called on a notifiable edge. Defaults to sendAll; overridden in
	// tests to observe the transition logic without the network. `ranFor` is
	// meaningful only for KindFinished — devices set their own threshold, so the
	// duration travels to the recipients rather than being judged here.
	notify func(k Kind, title, body, sessionID string, ranFor time.Duration)

	// now is time.Now, indirected so the finished-threshold logic is testable
	// without sleeping.
	now func() time.Time

	// send is one delivery attempt, indirected so RPCTest's synchronous
	// accounting can be asserted without a network (and without waiting out
	// pushTimeout against a fake endpoint). Defaults to sendOneResult.
	send func(webpush.Subscription, []byte) (int, error)
}

// SetTokenValidator installs the revocation authority: given the token identity a
// subscription recorded, it reports whether that credential still exists. The
// hub backs it with its live token store, so `workspacer token revoke` cuts a
// device's push the same way it cuts its bus access — no restart, no hand-edit.
// Subscriptions with no recorded identity are never passed here (see sendAll).
func (m *Manager) SetTokenValidator(fn func(tokenID string) bool) {
	m.mu.Lock()
	m.tokenValid = fn
	m.mu.Unlock()
}

// New loads (or generates) the VAPID keypair and loads any stored subscriptions
// from `dir`, creating it if needed. A generated keypair is persisted so the
// public key the phone subscribed against stays stable across hub restarts.
func New(dir string) (*Manager, error) {
	if dir == "" {
		return nil, errors.New("push: empty state dir")
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	m := &Manager{dir: dir, subs: map[string]storedSub{}, states: map[string]sessionState{}, now: time.Now}
	m.notify = m.sendAll
	m.send = m.sendOneResult
	if err := m.loadVAPID(); err != nil {
		return nil, err
	}
	m.loadSubs()
	return m, nil
}

func (m *Manager) vapidPath() string { return filepath.Join(m.dir, "vapid.json") }
func (m *Manager) subsPath() string  { return filepath.Join(m.dir, "push-subscriptions.json") }

func (m *Manager) loadVAPID() error {
	if data, err := os.ReadFile(m.vapidPath()); err == nil {
		var k vapidKeys
		if json.Unmarshal(data, &k) == nil && k.PublicKey != "" && k.PrivateKey != "" {
			m.vapidPub, m.vapidKey = k.PublicKey, k.PrivateKey
			return nil
		}
	}
	priv, pub, err := webpush.GenerateVAPIDKeys()
	if err != nil {
		return err
	}
	m.vapidPub, m.vapidKey = pub, priv
	blob, _ := json.Marshal(vapidKeys{PublicKey: pub, PrivateKey: priv})
	if err := os.WriteFile(m.vapidPath(), blob, 0o600); err != nil {
		return err
	}
	log.Printf("push: generated VAPID keypair at %s", m.vapidPath())
	return nil
}

func (m *Manager) loadSubs() {
	data, err := os.ReadFile(m.subsPath())
	if err != nil {
		return
	}
	var list []storedSub
	if json.Unmarshal(data, &list) != nil {
		return
	}
	for _, s := range list {
		if s.Endpoint != "" {
			m.subs[s.Endpoint] = s
		}
	}
	if len(m.subs) > 0 {
		log.Printf("push: loaded %d subscription(s)", len(m.subs))
	}
}

// persistSubs writes the current set. Caller must hold mu.
func (m *Manager) persistSubs() {
	list := make([]storedSub, 0, len(m.subs))
	for _, s := range m.subs {
		list = append(list, s)
	}
	blob, _ := json.Marshal(list)
	_ = os.WriteFile(m.subsPath(), blob, 0o600)
}

// PublicKey is the VAPID application-server key the client subscribes against.
func (m *Manager) PublicKey() string { return m.vapidPub }

// ── bus RPC handlers (registered as push.key / push.subscribe / push.unsubscribe) ──

// RPCKey returns { publicKey } so the client can build its pushManager subscription.
func (m *Manager) RPCKey(_ json.RawMessage) (any, error) {
	return map[string]string{"publicKey": m.vapidPub}, nil
}

// RPCSubscribe stores a browser PushSubscription ({ endpoint, keys:{p256dh, auth} })
// with no recorded subscriber. It exists for callers that can't see who is
// calling; prefer [Manager.RPCSubscribeAs], which records the identity that
// makes revocation possible.
func (m *Manager) RPCSubscribe(params json.RawMessage) (any, error) {
	return m.RPCSubscribeAs(Subscriber{}, params)
}

// RPCSubscribeAs stores a browser PushSubscription against the credential the
// calling connection presented. That identity is the whole point: a push
// registration outlives the connection that made it, so without it a revoked
// device keeps being notified for as long as the file survives.
func (m *Manager) RPCSubscribeAs(who Subscriber, params json.RawMessage) (any, error) {
	var s webpush.Subscription
	if err := json.Unmarshal(params, &s); err != nil {
		return nil, err
	}
	if s.Endpoint == "" || s.Keys.P256dh == "" || s.Keys.Auth == "" {
		return nil, errors.New("push.subscribe requires { endpoint, keys:{p256dh, auth} }")
	}
	// Per-device notification settings ride along on the same call the client
	// already makes on every connect, so the device stays the source of truth
	// for them without a second bus method (and a second thing to classify).
	// Absent `prefs` leaves every field unset, which reads as "all kinds, default
	// threshold" — the behaviour a client that predates this sees.
	var withPrefs struct {
		Prefs Prefs `json:"prefs"`
	}
	_ = json.Unmarshal(params, &withPrefs)
	// The endpoint is a NETWORK SINK the host is later made to POST to, on a
	// trigger the triage tier can pull at will. See endpoint.go.
	if err := validatePushEndpoint(s.Endpoint); err != nil {
		return nil, err
	}
	m.mu.Lock()
	m.subs[s.Endpoint] = storedSub{
		Subscription: s,
		TokenID:      who.TokenID,
		Scope:        who.Scope,
		Created:      time.Now().UTC().Truncate(time.Second),
		Prefs:        withPrefs.Prefs,
	}
	m.persistSubs()
	n := len(m.subs)
	m.mu.Unlock()
	log.Printf("push: subscription added (%d total)", n)
	return map[string]any{"ok": true}, nil
}

// RPCTest sends one notification to every registered device, right now.
//
// This exists because the alternative was a scavenger hunt: with no way to
// provoke a push, "I got nothing" could mean the trigger never fired, the
// delivery was filtered, the endpoint was stale, or the phone never registered
// — and telling those apart meant reading hub logs. It bypasses per-kind
// preferences by design (see KindTest) and reports how many devices it went to,
// so a zero is itself the answer: nothing is subscribed.
func (m *Manager) RPCTest(_ json.RawMessage) (any, error) {
	subs := m.recipients(KindTest, 0)
	payload, _ := json.Marshal(map[string]string{
		"title": "Workspacer", "body": "Test notification — push is working",
		"sessionId": "", "kind": string(KindTest),
	})

	// SYNCHRONOUS, unlike every other send. The first version reported
	// len(recipients) and fired goroutines, so it answered "how many rows are
	// stored" while appearing to answer "how many phones buzzed" — it said
	// "sent to 4 devices" while four dead endpoints were about to 410. The only
	// number worth returning is the one you get by waiting for it.
	statuses := make([]int, len(subs))
	var wg sync.WaitGroup
	for i, sub := range subs {
		wg.Add(1)
		go func(i int, sub webpush.Subscription) {
			defer wg.Done()
			status, err := m.send(sub, payload)
			if err != nil {
				status = 0
			}
			statuses[i] = status
		}(i, sub)
	}
	wg.Wait()

	delivered, gone, failed := 0, 0, 0
	for _, st := range statuses {
		switch {
		case st >= 200 && st <= 299:
			delivered++
		case st == 404 || st == 410:
			gone++
		default:
			failed++
		}
	}
	log.Printf("push: test — %d delivered, %d gone (pruned), %d failed, of %d stored",
		delivered, gone, failed, len(subs))
	return map[string]any{
		"ok": true, "devices": len(subs),
		"delivered": delivered, "gone": gone, "failed": failed,
	}, nil
}

// RPCUnsubscribe drops a subscription by endpoint, gated by proof-of-possession.
// The bus RPC layer carries no per-caller identity, so ownership is proven by
// presenting the subscription's `keys.auth` secret — a value the browser that
// created the subscription holds in its own PushSubscription (`toJSON().keys.auth`)
// but a caller with only the opaque endpoint cannot forge. If a subscription
// exists for the endpoint and has a non-empty stored auth, the request's auth
// must match it (constant-time). Unsubscribing an unknown endpoint is a no-op
// success so the call stays idempotent. Server-initiated pruning of dead
// subscriptions goes through removeEndpoint (from sendOne) and needs no auth.
func (m *Manager) RPCUnsubscribe(params json.RawMessage) (any, error) {
	var in struct {
		Endpoint string `json:"endpoint"`
		Auth     string `json:"auth"`
	}
	if err := json.Unmarshal(params, &in); err != nil {
		return nil, err
	}
	if in.Endpoint == "" {
		return map[string]any{"ok": true}, nil
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	sub, ok := m.subs[in.Endpoint]
	if !ok {
		return map[string]any{"ok": true}, nil // idempotent: nothing to drop
	}
	// Require proof-of-possession when the stored subscription carries an auth
	// secret (subscribe always stores one; the check is defensive if it doesn't).
	if sub.Keys.Auth != "" && subtle.ConstantTimeCompare([]byte(in.Auth), []byte(sub.Keys.Auth)) != 1 {
		return nil, errors.New("push.unsubscribe: auth does not match subscription")
	}
	delete(m.subs, in.Endpoint)
	m.persistSubs()
	return map[string]any{"ok": true}, nil
}

// RPCList reports every stored subscription with the identity that registered
// it and whether that identity still resolves. Operator surface: it appears in
// no scoped tier's method list, so view/triage tokens fail closed on it, and the
// endpoints it returns are what push.revoke takes. Without it, the set of
// devices being notified was only visible by reading a file on the hub host.
func (m *Manager) RPCList(_ json.RawMessage) (any, error) {
	stored, valid := m.snapshot()
	out := make([]map[string]any, 0, len(stored))
	for _, s := range stored {
		out = append(out, map[string]any{
			"endpoint": s.Endpoint,
			"tokenId":  s.TokenID,
			"scope":    s.Scope,
			"created":  s.Created,
			// A subscription whose token no longer resolves is already being
			// skipped by sendAll; surfacing it here is how an operator sees the
			// dead weight worth revoking.
			"revoked": s.TokenID != "" && valid != nil && !valid(s.TokenID),
		})
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i]["endpoint"].(string) < out[j]["endpoint"].(string)
	})
	return map[string]any{"subscriptions": out}, nil
}

// snapshot copies the stored subscriptions and the current validator. The
// validator is host-supplied and reads a file, so it is called on the copy with
// the lock released — nothing foreign runs under this mutex.
func (m *Manager) snapshot() ([]storedSub, func(string) bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]storedSub, 0, len(m.subs))
	for _, s := range m.subs {
		out = append(out, s)
	}
	return out, m.tokenValid
}

// RPCRevoke drops subscriptions by endpoint or by token identity, with no
// proof-of-possession — that gate exists on push.unsubscribe so a device can
// only retire its own registration, which is exactly the wrong shape when the
// device is the thing you're revoking. This is the operator's remedy (same
// tier-by-omission as push.list): revoke the credential, then revoke what it
// registered. Returns how many were removed; removing nothing is not an error.
func (m *Manager) RPCRevoke(params json.RawMessage) (any, error) {
	var in struct {
		Endpoint string `json:"endpoint"`
		TokenID  string `json:"tokenId"`
	}
	if err := json.Unmarshal(params, &in); err != nil {
		return nil, err
	}
	if in.Endpoint == "" && in.TokenID == "" {
		return nil, errors.New("push.revoke requires { endpoint } or { tokenId }")
	}
	m.mu.Lock()
	removed := 0
	for endpoint, s := range m.subs {
		if (in.Endpoint != "" && endpoint == in.Endpoint) || (in.TokenID != "" && s.TokenID == in.TokenID) {
			delete(m.subs, endpoint)
			removed++
		}
	}
	if removed > 0 {
		m.persistSubs()
	}
	m.mu.Unlock()
	if removed > 0 {
		log.Printf("push: revoked %d subscription(s)", removed)
	}
	return map[string]any{"ok": true, "removed": removed}, nil
}

func (m *Manager) removeEndpoint(endpoint string) {
	m.mu.Lock()
	if _, ok := m.subs[endpoint]; ok {
		delete(m.subs, endpoint)
		m.persistSubs()
	}
	m.mu.Unlock()
}

// ── snapshot watcher ────────────────────────────────────────────────────────

// Watch subscribes to agent.snapshot and fires a push on the un-blocked →
// blocked edge for each session. Runs until ctx is cancelled.
func (m *Manager) Watch(ctx context.Context, b *broker.Broker) {
	sub := b.Subscribe([]string{"agent.snapshot"})
	defer b.Unsubscribe(sub)
	for {
		select {
		case <-ctx.Done():
			return
		case ev := <-sub.C:
			m.onSnapshot(ev.Data)
		}
	}
}

// sessionState is what the watcher remembers between snapshots.
type sessionState struct {
	ambient string
	// workingSince is when this session most recently started working. Zero
	// when it is not (and has not been) working, which is what distinguishes a
	// genuine run from a session that was idle all along.
	workingSince time.Time
}

// blockedState reports whether an ambientState means "needs you".
func blockedState(s string) bool {
	return s == "waiting_approval" || s == "waiting_input"
}

// workingState reports whether an ambientState means the agent is doing
// something. `background` counts: the turn has ended but spawned subagents or a
// workflow are still running, so the work is not finished until that clears.
func workingState(s string) bool {
	return s == "thinking" || s == "streaming" || s == "background"
}

func (m *Manager) onSnapshot(data json.RawMessage) {
	var s struct {
		SessionID    string `json:"sessionId"`
		Cwd          string `json:"cwd"`
		LiveCwd      string `json:"liveCwd"`
		AmbientState string `json:"ambientState"`
		Status       string `json:"status"`
	}
	// ambientState is only present on the desktop-enriched snapshot (camelCase);
	// a brain/claudemon-backed raw snapshot lacks it and simply never fires.
	if json.Unmarshal(data, &s) != nil || s.SessionID == "" {
		return
	}
	name := dirName(s.LiveCwd)
	if name == "" {
		name = dirName(s.Cwd)
	}
	if name == "" {
		name = "Agent"
	}

	prev, seen := m.states[s.SessionID]

	if s.Status == "ended" {
		delete(m.states, s.SessionID)
		// Only for a session we were actually tracking: the snapshot list
		// carries ended rows for sessions that ended long ago (and the desktop
		// re-publishes them), so notifying on any ended row would ring the phone
		// for history every time a client reconnects.
		if seen {
			m.notify(KindEnded, name, "Session ended", s.SessionID, 0)
		}
		return
	}

	next := sessionState{ambient: s.AmbientState, workingSince: prev.workingSince}
	if workingState(s.AmbientState) {
		if !workingState(prev.ambient) || prev.workingSince.IsZero() {
			next.workingSince = m.now() // a run just started
		}
	}
	m.states[s.SessionID] = next

	if blockedState(s.AmbientState) && !blockedState(prev.ambient) {
		body := "Waiting for you"
		if s.AmbientState == "waiting_approval" {
			body = "Approve a tool use"
		} else if s.AmbientState == "waiting_input" {
			body = "Answer a question"
		}
		m.notify(KindNeeds, name+" needs you", body, s.SessionID, 0)
		return
	}

	// Finished: a run that was working (or parked waiting on you mid-run) has
	// gone idle. Blocked counts as still-in-the-run, so approving a tool and
	// walking away still earns the finish push.
	wasRunning := workingState(prev.ambient) || blockedState(prev.ambient)
	if s.AmbientState == "idle" && wasRunning && !prev.workingSince.IsZero() {
		ran := m.now().Sub(prev.workingSince)
		m.states[s.SessionID] = sessionState{ambient: s.AmbientState} // run is over
		m.notify(KindFinished, name+" finished", "Ran for "+humanDur(ran), s.SessionID, ran)
	}
}

// humanDur renders a run length the way a notification should read it —
// "40s", "4m", "1h12m" — not 2m3.004s.
func humanDur(d time.Duration) string {
	if d < time.Minute {
		return strconv.Itoa(int(d.Round(time.Second)/time.Second)) + "s"
	}
	if d < time.Hour {
		return strconv.Itoa(int(d.Round(time.Minute)/time.Minute)) + "m"
	}
	h := int(d / time.Hour)
	mins := int((d % time.Hour).Round(time.Minute) / time.Minute)
	if mins == 0 {
		return strconv.Itoa(h) + "h"
	}
	return strconv.Itoa(h) + "h" + strconv.Itoa(mins) + "m"
}

func dirName(p string) string {
	if p == "" {
		return ""
	}
	return filepath.Base(p)
}

// sendAll pushes to every still-authorized subscription concurrently. Payload
// is the JSON the service worker's `push` handler reads (title/body/sessionId).
//
// A subscription whose recorded credential no longer resolves is skipped, not
// deleted: the validator is backed by a file the hub re-reads, and that store
// fails closed on a transient read error — deleting on a momentary "no" would
// silently unsubscribe every device. Skipping makes revocation take effect
// immediately and reversibly; push.revoke is how an operator makes it permanent.
// A subscription with no recorded identity (registered before identity was
// tracked, or on a hub running without a token at all) has no credential to
// check and is delivered to.
//
// NOTE: within the authorized set this still broadcasts to ALL devices with no
// per-user filtering. That is intentional for the single-operator personal-tool
// model — one person, every device they've installed the /m PWA on should ring.
// A multi-user deployment would leak one operator's agent activity to another's
// devices; supporting that would mean filtering on the subscriber identity
// recorded here rather than only checking that it is still valid.
func (m *Manager) sendAll(k Kind, title, body, sessionID string, ranFor time.Duration) {
	payload, _ := json.Marshal(map[string]string{
		"title": title, "body": body, "sessionId": sessionID, "kind": string(k),
	})
	for _, s := range m.recipients(k, ranFor) {
		go m.sendOne(s, payload)
	}
}

// recipients is the authorized subset of the stored subscriptions that also
// WANT this kind — the revocation and preference filters, split out so they can
// be asserted on without a network.
//
// The threshold check lives here rather than at the trigger because it is a
// per-device setting: one phone can ask for every finish and another only for
// runs over five minutes, from the same edge.
func (m *Manager) recipients(k Kind, ranFor time.Duration) []webpush.Subscription {
	stored, valid := m.snapshot()
	subs := make([]webpush.Subscription, 0, len(stored))
	skipped, muted := 0, 0
	for _, s := range stored {
		if s.TokenID != "" && valid != nil && !valid(s.TokenID) {
			skipped++
			continue
		}
		if !s.Prefs.wants(k) {
			muted++
			continue
		}
		if k == KindFinished && ranFor < s.Prefs.finishedAfter() {
			muted++
			continue
		}
		subs = append(subs, s.Subscription)
	}
	if skipped > 0 {
		log.Printf("push: skipped %d subscription(s) whose token has been revoked", skipped)
	}
	if muted > 0 {
		log.Printf("push: %d subscription(s) have %q turned off or below threshold", muted, k)
	}
	return subs
}

// pushTimeout bounds a single push attempt. Without it these ran with no
// deadline at all: a push service reachable through a captive portal or a
// stalling VPN completes the TCP handshake and then never answers, so the
// goroutine blocks forever holding its socket — one more per stored
// subscription on every "needs you" transition, and shutdown can't reclaim them.
// A notification nobody could deliver in 10s has no value anyway; the next
// transition sends a fresh one.
const pushTimeout = 10 * time.Second

// pushClient is shared so attempts reuse connections and the idle ones are
// reaped, instead of each send building its own transport.
var pushClient = &http.Client{Timeout: pushTimeout}

func (m *Manager) sendOne(s webpush.Subscription, payload []byte) {
	_, _ = m.sendOneResult(s, payload)
}

// sendOneResult is sendOne with the outcome kept. Split out because the
// broadcast path genuinely does not care (it is fire-and-forget from a bus
// event) while the on-demand test does: "did it arrive" is the only question
// that one exists to answer.
//
// It also LOGS failures, which nothing did before. A push that never left the
// host was indistinguishable from one the phone ignored — the error was
// discarded on the line below and a non-2xx was silent unless it happened to be
// a 410. That silence is most of why "I got nothing" was unanswerable.
func (m *Manager) sendOneResult(s webpush.Subscription, payload []byte) (int, error) {
	ctx, cancel := context.WithTimeout(context.Background(), pushTimeout)
	defer cancel()
	resp, err := webpush.SendNotificationWithContext(ctx, payload, &s, &webpush.Options{
		HTTPClient:      pushClient,
		Subscriber:      vapidSubject,
		VAPIDPublicKey:  m.vapidPub,
		VAPIDPrivateKey: m.vapidKey,
		TTL:             60,
		Urgency:         webpush.UrgencyHigh,
	})
	if err != nil {
		log.Printf("push: send to %s failed: %v", endpointHost(s.Endpoint), err)
		return 0, err
	}
	defer resp.Body.Close()
	// The push service reports a dead subscription as Gone/Not Found — prune it
	// so we don't keep trying (and the store doesn't grow unbounded).
	if resp.StatusCode == 404 || resp.StatusCode == 410 {
		log.Printf("push: %s reports the subscription gone (%d) — pruning", endpointHost(s.Endpoint), resp.StatusCode)
		m.removeEndpoint(s.Endpoint)
		return resp.StatusCode, nil
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		log.Printf("push: %s refused the notification (%d)", endpointHost(s.Endpoint), resp.StatusCode)
	}
	return resp.StatusCode, nil
}

// endpointHost is the push service's host, for logs. The full endpoint embeds a
// per-device token, so it does not belong in a log line.
func endpointHost(endpoint string) string {
	if u, err := url.Parse(endpoint); err == nil && u.Host != "" {
		return u.Host
	}
	return "push service"
}
