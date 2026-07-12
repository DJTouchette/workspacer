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
	"os"
	"path/filepath"
	"sync"

	webpush "github.com/SherClockHolmes/webpush-go"
	"github.com/djtouchette/workspacer-hub/internal/broker"
)

// VAPID `sub` claim. Push services want a contact (mailto: or https URL); this
// is a stable placeholder — no mail is ever sent to it.
const vapidSubject = "mailto:workspacer@localhost"

type vapidKeys struct {
	PublicKey  string `json:"publicKey"`
	PrivateKey string `json:"privateKey"`
}

// Manager is safe for concurrent use. The snapshot watcher runs on one
// goroutine (so `states` needs no lock); `subs` is guarded by `mu`.
type Manager struct {
	dir      string
	vapidPub string
	vapidKey string

	mu   sync.Mutex
	subs map[string]webpush.Subscription // keyed by endpoint

	states map[string]string // sessionId -> last ambientState (watcher goroutine only)

	// notify is called on the un-blocked → blocked edge. Defaults to sendAll;
	// overridden in tests to observe the transition logic without the network.
	notify func(title, body, sessionID string)
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
	m := &Manager{dir: dir, subs: map[string]webpush.Subscription{}, states: map[string]string{}}
	m.notify = m.sendAll
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
	var list []webpush.Subscription
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
	list := make([]webpush.Subscription, 0, len(m.subs))
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

// RPCSubscribe stores a browser PushSubscription ({ endpoint, keys:{p256dh, auth} }).
func (m *Manager) RPCSubscribe(params json.RawMessage) (any, error) {
	var s webpush.Subscription
	if err := json.Unmarshal(params, &s); err != nil {
		return nil, err
	}
	if s.Endpoint == "" || s.Keys.P256dh == "" || s.Keys.Auth == "" {
		return nil, errors.New("push.subscribe requires { endpoint, keys:{p256dh, auth} }")
	}
	m.mu.Lock()
	m.subs[s.Endpoint] = s
	m.persistSubs()
	n := len(m.subs)
	m.mu.Unlock()
	log.Printf("push: subscription added (%d total)", n)
	return map[string]any{"ok": true}, nil
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

// blockedState reports whether an ambientState means "needs you".
func blockedState(s string) bool {
	return s == "waiting_approval" || s == "waiting_input"
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
	if s.Status == "ended" {
		delete(m.states, s.SessionID)
		return
	}
	prev := m.states[s.SessionID]
	m.states[s.SessionID] = s.AmbientState
	if blockedState(s.AmbientState) && !blockedState(prev) {
		name := dirName(s.LiveCwd)
		if name == "" {
			name = dirName(s.Cwd)
		}
		if name == "" {
			name = "Agent"
		}
		body := "Waiting for you"
		if s.AmbientState == "waiting_approval" {
			body = "Approve a tool use"
		} else if s.AmbientState == "waiting_input" {
			body = "Answer a question"
		}
		m.notify(name+" needs you", body, s.SessionID)
	}
}

func dirName(p string) string {
	if p == "" {
		return ""
	}
	return filepath.Base(p)
}

// sendAll pushes to every subscription concurrently. Payload is the JSON the
// service worker's `push` handler reads (title/body/sessionId).
//
// NOTE: this broadcasts every "needs you" push to ALL stored subscriptions with
// no per-user filtering. That is intentional for the single-operator personal-tool
// model — one person, every device they've installed the /m PWA on should ring.
// A multi-user deployment would leak one operator's agent activity to another's
// devices; supporting that would require tagging each subscription with an owner
// (or session scope) at subscribe time and filtering the recipient set here.
func (m *Manager) sendAll(title, body, sessionID string) {
	payload, _ := json.Marshal(map[string]string{"title": title, "body": body, "sessionId": sessionID})
	m.mu.Lock()
	subs := make([]webpush.Subscription, 0, len(m.subs))
	for _, s := range m.subs {
		subs = append(subs, s)
	}
	m.mu.Unlock()
	for _, s := range subs {
		go m.sendOne(s, payload)
	}
}

func (m *Manager) sendOne(s webpush.Subscription, payload []byte) {
	resp, err := webpush.SendNotification(payload, &s, &webpush.Options{
		Subscriber:      vapidSubject,
		VAPIDPublicKey:  m.vapidPub,
		VAPIDPrivateKey: m.vapidKey,
		TTL:             60,
		Urgency:         webpush.UrgencyHigh,
	})
	if err != nil {
		return
	}
	defer resp.Body.Close()
	// The push service reports a dead subscription as Gone/Not Found — prune it
	// so we don't keep trying (and the store doesn't grow unbounded).
	if resp.StatusCode == 404 || resp.StatusCode == 410 {
		m.removeEndpoint(s.Endpoint)
	}
}
