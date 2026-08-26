package bus

import (
	"encoding/json"
	"net/http"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// THE PROVIDER TIER, END TO END.
//
// Before it, `mayProvide` had two arms — `cn.trusted` and a PLUGIN MANIFEST —
// and cn.provides had exactly one source, so the only way to register a
// capability without a manifest was a token the bus promotes to `trusted`. A
// remote node is not a plugin (no manifest, no install-consent dialog, no
// per-capability fs roots), so the deployed Fly node held operator: nine
// authorities for a credential that needed one and a sliver of a second. With
// it, it could spawn agents, write config, wake and sleep billable machines,
// create host jobs, POST /plugins/install (clone a repo and run its build step
// ON THE HUB), forge agent.snapshot into the phone's push pipeline, and consume
// raw PTY bytes with ring-buffer replay for sessions on OTHER machines.
//
// These tests are the fence around that. Each one fails if the corresponding
// arm is reverted.

// providerTokenServer wires a bus with the real authtoken tiers plus a live
// provider record read from a real on-disk token file, exactly as
// cmd/hub/main.go wires it (ProvidesGrant, not the raw field).
func providerTokenServer(t *testing.T) (url string, srv *Server, tok string) {
	t.Helper()
	file := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := authtoken.Mint(file, authtoken.ScopeProvider, "fly-node")
	if err != nil {
		t.Fatal(err)
	}
	store := authtoken.NewStore(file)
	url, srv = rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		r, ok := store.Lookup(tok)
		if !ok {
			return ScopedIdent{}, false
		}
		return ScopedIdent{
			Scope:    string(r.Scope),
			Methods:  r.Scope.Methods(),
			Provides: r.ProvidesGrant(),
		}, true
	})
	return url, srv, rec.Token
}

// The name the bus matches on must be the name the CLI mints. internal/bus is
// a pure policy point and does not import internal/authtoken, so the two
// spellings are pinned here rather than shared — a rename on either side turns
// every provider connection into a tier the bus has never heard of, which fails
// closed as deny-all and looks exactly like a broken node.
func TestProviderScopeNameMatchesAuthtoken(t *testing.T) {
	if providerScope != string(authtoken.ScopeProvider) {
		t.Fatalf("bus.providerScope = %q, authtoken.ScopeProvider = %q — the twin drifted", providerScope, authtoken.ScopeProvider)
	}
}

// RISK 3, THE NEGATIVE. ScopedIdent.operator() scans Methods for "*" and reads
// a star there as HOST IDENTITY. A provider record's Provides is ["*"] by
// default. Nothing may merge the two lists — not the handshake, not a
// "simplification" of operator(), not a helper that concatenates grants — or
// every node token silently becomes the host token, which is strictly worse
// than the over-grant this tier exists to end.
func TestProviderTokenProvidesStarIsNotOperatorStar(t *testing.T) {
	si := ScopedIdent{
		Scope:    providerScope,
		Methods:  authtoken.ScopeProvider.Methods(),
		Provides: []string{"*"},
	}
	if si.operator() {
		t.Fatal("a provider ident with Provides=[\"*\"] reported operator() — the register grant is being read as host identity, so every node token is now trusted: nodes.wake, jobs.*, POST /plugins/install, the whole event firehose and the right to forge any host-owned topic")
	}

	// And on the wire, through the real handshake.
	url, srv, tok := providerTokenServer(t)
	c := dialClientToken(t, url, tok)
	if c.hello.Scope != providerScope {
		t.Fatalf("hello scope = %q, want %q — a provider promoted to operator would greet as operator", c.hello.Scope, providerScope)
	}
	if slices.Contains(c.hello.Methods, "*") {
		t.Fatalf("hello methods = %v — the register grant leaked into the call allowlist", c.hello.Methods)
	}
	if !slices.Equal(c.hello.Methods, []string{"layout.get"}) {
		t.Fatalf("hello methods = %v, want [layout.get]", c.hello.Methods)
	}

	// The identity the hub's own local handlers see (nodesTrusted, jobsTrusted)
	// must not be trusted.
	cn := connFor(t, srv, providerScope)
	id := cn.identity()
	if id.Trusted || id.IsTrusted() {
		t.Fatal("a provider connection reports IsTrusted() — nodesTrusted and the jobs.* gate both ask exactly that, so it may now spend money and create host jobs")
	}
	if id.Scope != providerScope {
		t.Fatalf("identity scope = %q, want %q", id.Scope, providerScope)
	}
}

// The tier's whole reason to exist: it REGISTERS, without being trusted.
func TestProviderTokenRegistersCapabilitiesWithoutHostAuthority(t *testing.T) {
	url, _, tok := providerTokenServer(t)
	node := dialClientToken(t, url, tok)

	// A representative slice of what `brain --scope full` registers.
	want := []string{
		"agents.list", "agents.spawn", "sessions.snapshot", "sessions.snapshots",
		"sessions.attachTerminal", "terminals.open", "claude.approve", "brain.info",
	}
	node.send(Frame{Op: "register", Methods: want})
	ack := node.readUntil("registered")
	if !slices.Equal(ack.Methods, want) {
		t.Fatalf("registered = %v, want all of %v. A provider token that cannot register is the tier failing at the one job it has — and a PARTIAL ack is worse than a refusal: the brain re-sends `register` every 5s forever, because the ack cannot say why a method was withheld.", ack.Methods, want)
	}

	// Registering it does NOT let it call it. The register grant is "may
	// answer", not "may ask" — the two are different verbs and different lists.
	for _, m := range []string{"agents.spawn", "claude.approve", "sessions.attachTerminal"} {
		node.send(Frame{Op: "call", ID: "self-" + m, Method: m})
		e := node.readUntil("error")
		if !strings.Contains(e.Error, "outside this token's") {
			t.Errorf("calling %q (which this connection PROVIDES) = %q, want a scope refusal. Providing a capability must not grant calling it, or a node could drive its own spawn surface.", m, e.Error)
		}
	}
}

// The call plane. The node's entire outbound surface is layout.get; everything
// the operator token used to carry is refused with an error naming the tier.
func TestProviderTokenCallSurfaceIsOneMethod(t *testing.T) {
	url, _, tok := providerTokenServer(t)

	// A trusted provider answers layout.get so the allowed case round-trips
	// through the real dispatch path rather than dying on "no provider".
	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "register", Methods: []string{"layout.get"}})
	host.readUntil("registered")
	go func() {
		for {
			f, ok := host.tryRead("call")
			if !ok {
				return
			}
			host.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
		}
	}()

	node := dialClientToken(t, url, tok)
	node.send(Frame{Op: "call", ID: "allowed", Method: "layout.get"})
	if r := node.readUntil("result"); r.ID != "allowed" {
		t.Fatalf("layout.get from a provider token: got id %q — the one method it needs must work", r.ID)
	}

	// Every authority the operator token used to hand it.
	for _, m := range []string{
		"nodes.wake",             // starts a billable machine
		"nodes.sleep",            // ends work in flight on one
		"agents.spawn",           // dispatch under this host's user
		"config.save",            // rewrite the host's config
		"jobs.upsert",            // create a recurring host job
		"layout.set",             // the write twin of the one read it holds
		"fs.write",               // the host filesystem
		"sessions.terminalInput", // raw keystrokes into a PTY
		"plugins.install",        // clone a repo and run its build step
		"sessions.snapshots",     // even a READ view holds: provider is not view
	} {
		node.send(Frame{Op: "call", ID: "deny-" + m, Method: m})
		e := node.readUntil("error")
		if e.ID != "deny-"+m {
			t.Fatalf("%s: correlation id = %q", m, e.ID)
		}
		if !strings.Contains(e.Error, "not authorized") || !strings.Contains(e.Error, providerScope) {
			t.Errorf("calling %q = %q, want a refusal naming the %q scope", m, e.Error, providerScope)
		}
	}
}

// THE PUBLISH GRANT — the one genuinely new authority in the tier. A provider
// may say a classified topic exactly when it registered the capability whose
// output that topic carries (capspec.EventTopic.Publisher). Nothing else.
func TestProviderTokenPublishesOnlyTheOutputOfWhatItProvides(t *testing.T) {
	url, _, tok := providerTokenServer(t)

	// A trusted watcher, so "did it publish" is answered by the broker rather
	// than by the absence of an error frame (publish is fire-and-forget).
	watcher := dialClientToken(t, url, "host-secret")
	watcher.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	watcher.readUntil("subscribed")

	node := dialClientToken(t, url, tok)
	node.send(Frame{Op: "register", Methods: []string{
		"sessions.snapshots", "sessions.snapshot", "sessions.attachTerminal", "terminals.open",
	}})
	node.readUntil("registered")

	// The four topics cmd/brain actually publishes, verified against its real
	// publish sites. Without these the node comes up MUTE: no fleet feed, no
	// statuslines, no PTY stream, no visible-terminal requests.
	for _, typ := range []string{
		"agent.snapshot",      // cmd/brain/main.go
		"agent.statusline",    // cmd/brain/events.go
		"pty.bytes.sess-1",    // cmd/brain/terminal.go
		"facade.openTerminal", // cmd/brain/visibleterm.go
	} {
		node.send(Frame{Op: "publish", Event: &event.Envelope{
			Type: typ, Data: json.RawMessage(`{"marker":"from-node"}`)}})
		got, ok := watcher.tryReadUntil("event", "event", 2*time.Second)
		if !ok || got.Event == nil || got.Event.Type != typ {
			t.Fatalf("a provider that registered the backing capability could not publish %q — the headless node is mute and the fleet feed is dead", typ)
		}
	}

	// And the refusals. Each of these is host state no provider answers for:
	// a forged layout.changed carries the four spawn-escalation fields
	// layout.set scrubs (plus a version that wins every later comparison); a
	// forged plugin.settings.changed discloses plugin endpoints and paths;
	// an unclassified topic is the manifest plane, which a token has no
	// manifest for.
	for _, typ := range []string{
		"layout.changed",
		"plugin.settings.changed",
		"plugin.log",
		"node.state_changed",
		"agent.state_changed",
		"fs.changed", // guarded by fs.watch, which this node did not register
		"example.clock.tick",
	} {
		node.send(Frame{Op: "publish", Event: &event.Envelope{
			Type: typ, Data: json.RawMessage(`{"marker":"forged"}`)}})
	}
	// A publish the bus refuses produces no event at all. One trailing publish
	// the node IS allowed makes this a positive assertion rather than a race
	// against nothing: if any forgery landed, it arrives before the sentinel.
	node.send(Frame{Op: "publish", Event: &event.Envelope{
		Type: "agent.snapshot", Data: json.RawMessage(`{"marker":"sentinel"}`)}})
	got, ok := watcher.tryReadUntil("event", "event", 3*time.Second)
	if !ok || got.Event == nil {
		t.Fatal("the sentinel publish never arrived — the test cannot tell a refusal from a lost frame")
	}
	if !strings.Contains(string(got.Event.Data), "sentinel") {
		t.Fatalf("a provider token forged %q — host state is publishable by a credential that does not answer for it", got.Event.Type)
	}
}

// The publish grant follows the token record's `provides` GRANT, not the tier
// and not the live router slot. A provider whose grant was narrowed below a
// capability may not publish that capability's output, even at the same tier
// and on the same socket that publishes its other topics fine.
//
// Grant rather than ownership is deliberate: the router is
// first-registration-wins, so on a hub that also has a desktop the desktop owns
// sessions.snapshots and the node's register of it is withheld — under an
// ownership test that node would go silently MUTE while still answering
// everything else. See mayPublish.
func TestProviderPublishGrantFollowsProvidesNotTheTier(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		if tok != "tok-node" {
			return ScopedIdent{}, false
		}
		return ScopedIdent{
			Scope:   providerScope,
			Methods: authtoken.ScopeProvider.Methods(),
			// Narrowed: the fleet read, and nothing that owns a terminal.
			Provides: []string{"sessions.snapshots"},
		}, true
	})

	watcher := dialClientToken(t, url, "host-secret")
	watcher.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	watcher.readUntil("subscribed")

	node := dialClientToken(t, url, "tok-node")
	// Registering it does not buy it either — the grant is the answer.
	node.send(Frame{Op: "register", Methods: []string{"sessions.attachTerminal", "terminals.open"}})
	if ack := node.readUntil("registered"); len(ack.Methods) != 0 {
		t.Fatalf("a narrowed grant registered %v — mayProvide is not consulting the record's provides", ack.Methods)
	}

	node.send(Frame{Op: "publish", Event: &event.Envelope{
		Type: "pty.bytes.sess-1", Data: json.RawMessage(`{"marker":"forged"}`)}})
	node.send(Frame{Op: "publish", Event: &event.Envelope{
		Type: "facade.openTerminal", Data: json.RawMessage(`{"marker":"forged"}`)}})
	node.send(Frame{Op: "publish", Event: &event.Envelope{
		Type: "agent.snapshot", Data: json.RawMessage(`{"marker":"sentinel"}`)}})

	got, ok := watcher.tryReadUntil("event", "event", 3*time.Second)
	if !ok || got.Event == nil {
		t.Fatal("the sentinel publish never arrived — a provider must still publish the output of what it DOES provide")
	}
	if got.Event.Type != "agent.snapshot" {
		t.Fatalf("published %q with a grant that does not include sessions.attachTerminal or terminals.open. The publish grant is coming from the TIER rather than from the record, so narrowing a node's grant would leave it able to stream any session's PTY bytes at every subscriber.", got.Event.Type)
	}
}

// A plugin's manifest cannot buy the publish side of a core capability.
// internal/plugin's validateProvides confines a manifest's `provides` to the
// plugin's own namespace, so this is unreachable through the manager — pinned
// here because the bus is the layer that must hold even if that confinement
// were ever loosened.
func TestPluginNamespaceProvidesCannotPublishCoreTopics(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.RegisterPluginToken("plug-tok", "acme", []capspec.Grant{}, capspec.EventGrants{
		Provides: []string{"acme.*"},
		Emits:    []string{"acme.tick", "pty.bytes.*", "agent.snapshot"},
	})
	watcher := dialClientToken(t, url, "host-secret")
	watcher.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	watcher.readUntil("subscribed")

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.s1"}})
	plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.snapshot"}})
	plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: "acme.tick"}})

	got, ok := watcher.tryReadUntil("event", "event", 3*time.Second)
	if !ok || got.Event == nil {
		t.Fatal("the plugin's own unclassified topic never arrived — the manifest plane broke")
	}
	if got.Event.Type != "acme.tick" {
		t.Fatalf("a plugin whose provides is confined to its own namespace published %q. The classified plane is host state: a manifest `emits` is a filter there, never a grant.", got.Event.Type)
	}
}

// CONSUMES NOTHING. Hard false, before any topic is classified — including
// topics guarded by a capability this connection provides. The symmetric rule
// was available and refused: it would let a node that provides
// sessions.attachTerminal consume pty.bytes.* for sessions on the DESKTOP.
func TestProviderTokenReceivesNoEventsAtAll(t *testing.T) {
	url, _, tok := providerTokenServer(t)

	node := dialClientToken(t, url, tok)
	node.send(Frame{Op: "register", Methods: []string{
		"sessions.snapshots", "sessions.snapshot", "sessions.attachTerminal", "terminals.open",
	}})
	node.readUntil("registered")
	node.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	node.readUntil("subscribed")

	// A view token subscribed alongside is the FLOOR: it proves the publishes
	// below really are reaching subscribers, so "the provider got nothing" is a
	// refusal and not a dead broker.
	srvLookup := dialClientToken(t, url, "host-secret")
	srvLookup.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	srvLookup.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	for _, typ := range []string{
		"agent.snapshot",      // open-by-decision: every human tier receives it
		"agent.state_changed", // ditto
		"pty.bytes.sess-1",    // guarded by a capability this node PROVIDES
		"agent.statusline",    // ditto
		"facade.openTerminal", // ditto
		"command.focus_agent", // open-by-decision
		"hub.peer.connected",  // open-by-decision
		"example.clock.tick",  // unclassified
	} {
		host.send(Frame{Op: "publish", Event: &event.Envelope{
			Type: typ, Data: json.RawMessage(`{"marker":"fleet"}`)}})
	}
	if _, ok := srvLookup.tryReadUntil("event", "event", 2*time.Second); !ok {
		t.Fatal("floor: a trusted subscriber received nothing either — the publishes never landed and this test proves nothing")
	}
	if got, ok := node.tryReadUntil("event", "event", 1500*time.Millisecond); ok {
		t.Fatalf("a provider token received %q. It answers calls; it does not watch the fleet — and PROVIDING a capability is not permission to WATCH its output on somebody else's machine.", got.Event.Type)
	}
}

// The HTTP plane. Server.Authorized gates POST /plugins/install — clone a repo
// and run its build step on the hub — plus /plugins/remove, /plugins/settings
// and the /app entry. A provider token must fail it STRUCTURALLY, not because
// a point guard names it.
func TestProviderTokenIsNotAuthorizedForGuardedHTTPRoutes(t *testing.T) {
	_, srv, tok := providerTokenServer(t)
	req := func(tok, path string) *http.Request {
		r, _ := http.NewRequest(http.MethodPost, path, nil)
		r.Header.Set("Authorization", "Bearer "+tok)
		return r
	}
	for _, path := range []string{"/plugins/install", "/plugins/remove", "/plugins/settings", "/app"} {
		if srv.Authorized(req(tok, path)) {
			t.Errorf("a provider token passed Server.Authorized for %s — that is the guard on cloning a repo and running its build step ON THE HUB", path)
		}
	}
	if !srv.Authorized(req("host-secret", "/plugins/install")) {
		t.Fatal("floor: the host token must still pass, or this test proves only that the guard is broken for everyone")
	}
}

// RISK 2. revalidateScoped compares the live record against the handshake
// snapshot every tick and closes the socket on drift. Provides is a new
// per-token grant snapshotted at handshake, so it must join that comparison —
// omit it and NARROWING a node's register grant applies to every future
// connection and to nothing that is currently answering, which is the one
// situation narrowing is for.
func TestNarrowingAProviderTokensRegisterGrantClosesItsLiveSocket(t *testing.T) {
	restore := shortenScopedRevalidation(t)
	defer restore()

	// Read by the server's revalidation goroutine while this test writes it.
	var grant atomic.Value
	grant.Store([]string{"*"})
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		if tok != "tok-node" {
			return ScopedIdent{}, false
		}
		return ScopedIdent{
			Scope:    providerScope,
			Methods:  authtoken.ScopeProvider.Methods(),
			Provides: grant.Load().([]string),
		}, true
	})

	node := dialClientToken(t, url, "tok-node")
	node.send(Frame{Op: "register", Methods: []string{"claude.approve"}})
	if ack := node.readUntil("registered"); len(ack.Methods) != 1 {
		t.Fatalf("floor: the wide grant must register claude.approve, got %v", ack.Methods)
	}

	// Narrow the record. The tier did not change and the token still resolves,
	// so a "does this token still exist" re-check misses this entirely.
	grant.Store([]string{"brain.info"})

	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		node.send(Frame{Op: "call", ID: "probe", Method: "layout.get"})
		if _, ok := node.tryReadUntil("error", "result", 200*time.Millisecond); !ok {
			node.ws.CloseNow()
			return // socket closed — the narrowing took effect
		}
	}
	t.Fatal("a provider token whose `provides` was narrowed kept its wide socket. It is still the registered answerer of every capability the old grant matched — every subsequent caller's params (prompts, file contents, approval decisions) keep routing to it — until the hub process restarts.")
}

// mayProvide had no revocation check while mayCall and mayConsume both did, and
// mayProvide is the one whose effect OUTLIVES the frame: first-registration-
// wins means a revoked socket that claims a capability slot keeps receiving
// every subsequent caller's params until it actually drops.
func TestRevokedProviderCannotClaimACapabilitySlot(t *testing.T) {
	url, srv, tok := providerTokenServer(t)
	node := dialClientToken(t, url, tok)
	node.send(Frame{Op: "register", Methods: []string{"brain.info"}})
	if ack := node.readUntil("registered"); len(ack.Methods) != 1 {
		t.Fatalf("floor: registration must work before revocation, got %v", ack.Methods)
	}

	cn := connFor(t, srv, providerScope)
	cn.revoked.Store(true)
	if cn.mayProvide("claude.approve") {
		t.Fatal("a REVOKED provider connection may still register claude.approve — first-registration-wins makes that claim outlive the revocation, and every approval decision routes to it until the socket drops")
	}
	if cn.mayPublish("agent.snapshot") {
		t.Fatal("a revoked provider connection may still publish — mayPublish's grant runs through mayProvide, so the revocation must reach both")
	}
}

// connFor finds the single live connection at the given scope. Tests reach into
// the router because the properties being pinned (identity, revocation) are not
// observable from the wire without also asserting the thing under test.
func connFor(t *testing.T, srv *Server, scope string) *conn {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		srv.router.mu.Lock()
		var found *conn
		n := 0
		for _, c := range srv.router.conns {
			if c.scope == scope {
				found, n = c, n+1
			}
		}
		srv.router.mu.Unlock()
		if n == 1 {
			return found
		}
		if n > 1 {
			t.Fatalf("%d connections at scope %q — connFor cannot pick one", n, scope)
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("no live connection at scope %q", scope)
	return nil
}
