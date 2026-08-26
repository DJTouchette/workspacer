// Package federation links this hub to named peer hubs (hub-of-hubs).
//
// Each configured peer gets an outbound busclient — the same client a plugin
// sidecar or the MCP facade uses — subscribed to a CURATED topic list. Inbound
// events are stamped with the peer's name (Envelope.Hub) and republished onto
// the local broker with their payload bytes untouched, so every local client
// (renderer, /m, plugins, the supervisor) sees one fleet spanning machines
// while still talking to exactly one bus. See docs/hub-federation.md.
//
// Topology is a TREE by construction: an event that already carries a Hub
// stamp arrived over someone else's federation link and is dropped, never
// re-forwarded — loops are unrepresentable rather than solved. The peer needs
// no concept of federation at all; it sees an ordinary scoped client, and the
// scoped token it minted is the ceiling on everything this link can do.
//
// The forward list is deliberately an allowlist, not `*`. Subscribing to
// everything would launder the peer's control plane into ours: its
// layout.changed would clobber the local shared-layout document, its host-only
// plugin.* topics carry manifests and secret-bearing stderr, and its command.*
// events would DRIVE THIS MACHINE'S UI. A topic joins the list deliberately or
// not at all.
package federation

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/redact"
)

// ForwardTopics is the curated set of peer topics republished locally: the
// fleet feed and workflow progress — what a unified fleet view needs, and
// nothing that mutates local state or steers the local UI.
var ForwardTopics = []string{"agent.*", "workflow.*"}

// pollInterval is how often each link checks its connection state for the
// connected/disconnected transitions (busclient exposes Ready, not a status
// callback).
const pollInterval = time.Second

// Peer is one configured upstream hub.
type Peer struct {
	Name  string // the label stamped on forwarded envelopes; must be unique
	URL   string // ws://host:7895/bus
	Token string // a scoped token minted BY THE PEER (workspacer token create)
}

// ParsePeerFlag parses one repeatable -peer value:
//
//	name=work,url=ws://host:7895/bus,token=abc123
//
// Order-insensitive; name and url are required. The name becomes part of
// qualified method names (`hub:<name>/agents.list`) and of client-side state
// keys, so it is confined to a safe shape.
func ParsePeerFlag(s string) (Peer, error) {
	var p Peer
	for _, part := range strings.Split(s, ",") {
		k, v, ok := strings.Cut(strings.TrimSpace(part), "=")
		if !ok {
			return Peer{}, fmt.Errorf("peer %q: %q is not key=value", s, part)
		}
		switch strings.ToLower(strings.TrimSpace(k)) {
		case "name":
			p.Name = strings.TrimSpace(v)
		case "url":
			p.URL = strings.TrimSpace(v)
		case "token":
			p.Token = strings.TrimSpace(v)
		default:
			return Peer{}, fmt.Errorf("peer %q: unknown key %q (want name, url, token)", s, k)
		}
	}
	if p.Name == "" || p.URL == "" {
		return Peer{}, fmt.Errorf("peer %q: name and url are required", s)
	}
	if !validPeerName(p.Name) {
		return Peer{}, fmt.Errorf("peer name %q: use letters, digits, - or _ (it becomes part of method names and state keys)", p.Name)
	}
	if !strings.HasPrefix(p.URL, "ws://") && !strings.HasPrefix(p.URL, "wss://") {
		return Peer{}, fmt.Errorf("peer %q: url must be ws:// or wss://", p.Name)
	}
	return p, nil
}

// validPeerName confines a peer name to `[A-Za-z0-9_-]+` — it is interpolated
// into qualified method names and event payloads, so it must never carry the
// `/` or `:` the qualification syntax uses, nor whitespace.
func validPeerName(name string) bool {
	if name == "" {
		return false
	}
	for _, r := range name {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9', r == '-', r == '_':
		default:
			return false
		}
	}
	return true
}

// Publisher is the slice of the broker federation needs.
type Publisher interface {
	Publish(event.Envelope)
}

// Manager owns every federation link.
type Manager struct {
	pub   Publisher
	links []*link
}

// link is one peer connection: the busclient, its state, and its counters.
type link struct {
	peer   Peer
	client *busclient.Client
	pub    Publisher

	mu        sync.Mutex
	connected bool
	lastSeen  time.Time // last moment the link was observed connected
	forwarded uint64
	dropped   uint64 // events refused by the tree invariant (already stamped)
}

// New builds a manager for the configured peers. Duplicate names are refused —
// they would silently merge two machines' fleets under one label.
func New(pub Publisher, peers []Peer) (*Manager, error) {
	m := &Manager{pub: pub}
	seen := map[string]bool{}
	for _, p := range peers {
		if seen[p.Name] {
			return nil, fmt.Errorf("duplicate peer name %q", p.Name)
		}
		seen[p.Name] = true
		l := &link{peer: p, client: busclient.New(peerLinkURL(p.URL), p.Token), pub: pub}
		l.client.OnEvent(l.forward)
		l.client.Subscribe(ForwardTopics...)
		m.links = append(m.links, l)
	}
	return m, nil
}

// peerLinkURL tags a peer dial with bus.PeerLinkParam so the FAR hub can tell a
// federation link from an ordinary client holding the same token.
//
// It exists for one asymmetry: a forwarded `hub:<peer>/agents.spawn` re-enters
// the peer's router on THIS link's connection, and peers.json routinely holds
// the peer's host token — so "the link is authenticated" would otherwise mean
// "anything this link forwards runs with host authority", including a
// permission bypass the originating caller was never granted. The far hub reads
// the tag only to WITHHOLD (see bus.conn.mayBypassPermissions), so a link that
// forgot it is not an escalation door and a client that fakes it only limits
// itself.
func peerLinkURL(u string) string {
	sep := "?"
	if strings.Contains(u, "?") {
		sep = "&"
	}
	return u + sep + bus.PeerLinkParam + "=1"
}

// Peers returns the configured peer names, for validation of qualified calls.
func (m *Manager) Peers() []string {
	out := make([]string, 0, len(m.links))
	for _, l := range m.links {
		out = append(out, l.peer.Name)
	}
	return out
}

// Client returns the busclient for a named peer (nil if unknown) — the
// forwarding channel for qualified capability calls.
func (m *Manager) Client(peer string) *busclient.Client {
	for _, l := range m.links {
		if l.peer.Name == peer {
			return l.client
		}
	}
	return nil
}

// Run starts every link and blocks until ctx ends.
func (m *Manager) Run(ctx context.Context) {
	var wg sync.WaitGroup
	for _, l := range m.links {
		wg.Add(1)
		go func(l *link) {
			defer wg.Done()
			l.run(ctx)
		}(l)
	}
	wg.Wait()
}

// forward is the republish path: stamp the peer name, hand the bytes to the
// local broker. Runs on the busclient read loop, so it must stay cheap —
// broker.Publish is a non-blocking fan-out.
func (l *link) forward(ev event.Envelope) {
	if ev.Hub != "" {
		// Already federated once: this is the peer's own upstream traffic.
		// Forwarding it would make loops representable; the tree invariant
		// says drop, and the counter keeps the drop visible.
		l.mu.Lock()
		l.dropped++
		l.mu.Unlock()
		return
	}
	ev.Hub = l.peer.Name
	// Clear the peer's broker-assigned id so the LOCAL broker assigns its own:
	// id spaces are per-broker, and a forwarded id could collide with a local
	// one in clients that key dedupe on it.
	ev.ID = ""
	l.mu.Lock()
	l.forwarded++
	l.mu.Unlock()
	l.pub.Publish(ev)
}

// run watches the link's connection state and publishes peer lifecycle events
// on transitions. The client itself reconnects with backoff forever; this loop
// only narrates.
func (l *link) run(ctx context.Context) {
	go l.client.Run(ctx)
	t := time.NewTicker(pollInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
		}
		up := l.client.Ready()
		l.mu.Lock()
		was := l.connected
		if up {
			l.lastSeen = time.Now()
		}
		lastSeen := l.lastSeen
		l.connected = up
		l.mu.Unlock()
		if up == was {
			continue
		}
		if up {
			log.Printf("federation: peer %q connected (%s)", l.peer.Name, redact.URL(l.peer.URL))
			l.pub.Publish(event.New("hub.peer.connected", "federation", map[string]any{
				"peer": l.peer.Name,
			}))
		} else {
			log.Printf("federation: peer %q disconnected (%s)", l.peer.Name, redact.URL(l.peer.URL))
			l.pub.Publish(event.New("hub.peer.disconnected", "federation", map[string]any{
				"peer":     l.peer.Name,
				"lastSeen": lastSeen.UTC().Format(time.RFC3339),
			}))
		}
	}
}

// forwardTimeout is the federated hop's budget — deliberately shorter than the
// bus router's 30s callTimeout, so when the peer side stalls the caller sees
// THIS hop's failure rather than an ambiguous local deadline racing it.
const forwardTimeout = 25 * time.Second

// HasPeer implements bus.Federation.
func (m *Manager) HasPeer(name string) bool {
	return m.Client(name) != nil
}

// Forward implements bus.Federation: invoke the bare method on the peer over
// its link and return the raw result. The peer authorizes the call against the
// LINK token's scope — that grant is the ceiling on everything forwarded here,
// whoever the local caller was.
func (m *Manager) Forward(ctx context.Context, peer, method string, params json.RawMessage) (json.RawMessage, error) {
	c := m.Client(peer)
	if c == nil {
		return nil, fmt.Errorf("unknown federation peer %q", peer)
	}
	ctx, cancel := context.WithTimeout(ctx, forwardTimeout)
	defer cancel()
	return c.Call(ctx, method, params)
}

// DefaultPeersPath is where peers persist: <config>/workspacer/peers.json,
// next to tokens.json. A separate file, deliberately, twice over: peer entries
// carry BEARER TOKENS for other machines, and config.yaml is credential-free
// by design (that is what keeps config.get unguarded) — and a -peer flag would
// put the token in argv, which /proc/<pid>/cmdline makes world-readable. The
// flag form still exists for tests and throwaway dev links; durable peers
// belong here.
func DefaultPeersPath() string {
	return filepath.Join(authtoken.ConfigDir(), "peers.json")
}

// LoadPeersFile reads peers.json: a JSON array of {name,url,token}. A missing
// file is no peers; a corrupt file is an error (fail loudly — a typo silently
// disabling federation reads as "my other machine vanished").
func LoadPeersFile(path string) ([]Peer, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var raw []struct {
		Name  string `json:"name"`
		URL   string `json:"url"`
		Token string `json:"token,omitempty"`
	}
	if err := json.Unmarshal(b, &raw); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	out := make([]Peer, 0, len(raw))
	for _, r := range raw {
		p := Peer{Name: strings.TrimSpace(r.Name), URL: strings.TrimSpace(r.URL), Token: r.Token}
		if p.Name == "" || p.URL == "" {
			return nil, fmt.Errorf("%s: peer entries need name and url", path)
		}
		if !validPeerName(p.Name) {
			return nil, fmt.Errorf("%s: peer name %q: use letters, digits, - or _", path, p.Name)
		}
		if !strings.HasPrefix(p.URL, "ws://") && !strings.HasPrefix(p.URL, "wss://") {
			return nil, fmt.Errorf("%s: peer %q url must be ws:// or wss://", path, p.Name)
		}
		out = append(out, p)
	}
	return out, nil
}

// PeerInfo is one peer's liveness, as served by the hub-local
// `federation.peers` method (the web renderer's substitute for the desktop's
// federation:peers IPC — a browser can't read peers.json).
type PeerInfo struct {
	Name      string `json:"name"`
	Connected bool   `json:"connected"`
	// LastSeen is unix milliseconds of the last observed liveness; 0 = never.
	LastSeen int64 `json:"lastSeen,omitempty"`
}

// PeersInfo reports every configured peer's current link state.
func (m *Manager) PeersInfo() []PeerInfo {
	out := make([]PeerInfo, 0, len(m.links))
	for _, l := range m.links {
		l.mu.Lock()
		info := PeerInfo{Name: l.peer.Name, Connected: l.connected}
		if !l.lastSeen.IsZero() {
			info.LastSeen = l.lastSeen.UnixMilli()
		}
		l.mu.Unlock()
		out = append(out, info)
	}
	return out
}
