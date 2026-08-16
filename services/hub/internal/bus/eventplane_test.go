package bus

import (
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// THE MATRIX, and its ANSWER KEY.
//
// Round 6 fixed two topics by name and left the completeness question open; a
// scratch run of this shape answered it — 23 of 25 published topics were
// delivered to a `view` token, and the guard table had two rows. This is that
// run, kept.
//
// THE TEST THIS REPLACES WAS A TAUTOLOGY. It published every registry pattern
// and then asserted `switch row.Disposition { case TopicHostOnly: if got {fail}
// …}` — the expectation was READ OFF the row it was supposed to be checking, so
// flipping a row flipped the expectation with it and the assertion still passed.
// Thirteen of the eighteen rows were unanchored by it: agent.statusline,
// sidecar.*, plugin.settings.changed, plugin.loaded and plugin.install.progress
// could each be moved to TopicOpenByDecision with the entire Go suite green.
// plugin.loaded carries install argv, the sidecar's server command and every
// declared filesystem scope; its own row argues it is the most sensitive payload
// in the table, and nothing pinned it.
//
// So the expectation is written HERE, by hand, from each topic's PAYLOAD, and
// the credentials are chosen so that the three dispositions are distinguishable
// by delivery alone. That last part is what the view tier alone cannot do:
//
//	bare   a plugin whose manifest consumes "*" holding NO capability. It
//	       receives open-by-decision topics and unclassified ones and nothing
//	       else, so `bare` is exactly the open/not-open bit.
//	term   the same plugin holding ONLY sessions.attachTerminal
//	watch  … ONLY fs.watch
//	snap   … ONLY sessions.snapshot
//	       Three single-capability columns, so a guarded row is pinned to the
//	       METHOD that unlocks it: retargeting agent.statusline at fs.watch moves
//	       its delivery from one column to another and fails by name.
//	view   the real authtoken.ScopeView tier — the credential every leak in this
//	       file was found on. It HOLDS sessions.snapshot and does not hold
//	       sessions.attachTerminal or fs.watch, which is why on its own it cannot
//	       tell guarded-by-sessions.snapshot from open.
//	op     the operator tier, trusted: it must receive everything, so a blanket
//	       denial wearing a table fails here too.
//
// A host-only row is the one shape where every column except `op` is false; an
// open row is every column true. Neither can be reached by editing eventtopics.go
// alone.
type topicDelivery struct {
	topic string // the CONCRETE topic as it goes on the wire
	view  bool
	bare  bool
	term  bool
	watch bool
	snap  bool
	why   string
}

// unclassifiedProbe is deliberately in no registry row: the plane's default.
const unclassifiedProbe = "invented.topic.nobody.classified"

var topicDeliveryKey = []topicDelivery{
	// ---- the PTY family: the output of sessions.attachTerminal --------------
	{topic: "pty.bytes.SECRET-42", term: true,
		why: "raw PTY bytes with the ring-buffer replay — the output side of sessions.terminalInput, which no scoped tier holds"},
	{topic: "pty.exit", term: true,
		why: "end-of-stream for that same guarded stream, naming the sessionId"},
	{topic: "pty.desync", term: true,
		why: "synthesized from a DROPPED pty.bytes.<id>, so its payload is the identity of a stream this connection was refused"},

	// ---- filesystem --------------------------------------------------------
	{topic: "fs.changed", watch: true,
		why: "a change feed on a path — an activity oracle on files whose contents may be unreadable. fs.watch installs the watcher and is in no scoped tier"},

	// ---- the fleet feed ----------------------------------------------------
	// view TRUE here and nowhere else among the guarded rows: ScopeView holds
	// sessions.snapshot, and status_line is merged into that snapshot.
	{topic: "agent.statusline", view: true, snap: true,
		why: "status_line merged into the session snapshot — id, model, cost_usd and rate-limit state. sessions.snapshot is the capability that returns it, and the view tier holds THAT and not the others"},
	{topic: "agent.snapshot", view: true, bare: true, term: true, watch: true, snap: true,
		why: "the fleet feed the view tier exists for, filtered by the same vis.visible rule as sessions.snapshots before publication"},
	{topic: "agent.state_changed", view: true, bare: true, term: true, watch: true, snap: true,
		why: "{sessionId, hookEvent, mode, cwd} — a strict subset of the agent.snapshot the same tier already gets, and the wake signal every remote client needs"},
	{topic: "layout.changed", view: true, bare: true, term: true, watch: true, snap: true,
		why: "the accepted shared layout document, whose read (layout.get) is in the view tier. The danger here was forgery, closed by mayPublish, not disclosure"},

	// ---- federation-link reachability ---------------------------------------
	{topic: "hub.peer.connected", view: true, bare: true, term: true, watch: true, snap: true,
		why: "only the peer's configured name; the feed it enables (agent.*) is already open to these credentials"},
	{topic: "hub.peer.disconnected", view: true, bare: true, term: true, watch: true, snap: true,
		why: "peer name + last-seen — the tombstone signal; withholding it makes remote agents silently vanish for exactly the clients that watch them"},

	// ---- UI navigation commands (MCP facade ui tools → the renderer) --------
	{topic: "command.focus_agent", view: true, bare: true, term: true, watch: true, snap: true,
		why: "a navigation REQUEST carrying only the session id to focus — an id the fleet feed already delivers to every one of these credentials"},
	{topic: "command.open_pane", view: true, bare: true, term: true, watch: true, snap: true,
		why: "a pane type plus optional cwd/url chosen by the publisher; receiving it discloses only that navigation was requested"},
	{topic: "command.open_plugin", view: true, bare: true, term: true, watch: true, snap: true,
		why: "an installed plugin's pane type, already listed by the unauthenticated /plugins projection"},
	{topic: "command.open_spawn_dialog", view: true, bare: true, term: true, watch: true, snap: true,
		why: "an optional directory to pre-fill in the New Agent dialog; the spawn itself still goes through agents.spawn and its clamps"},
	{topic: "workflow.started", view: true, bare: true, term: true, watch: true, snap: true,
		why: "run name, phases, agents and a cwd the tier already has via agent.snapshot"},
	{topic: "workflow.completed", view: true, bare: true, term: true, watch: true, snap: true,
		why: "the completion half of workflow.started"},
	{topic: "workflow.failed", view: true, bare: true, term: true, watch: true, snap: true,
		why: "the failure half of workflow.started"},
	{topic: "workflow.agent.finished", view: true, bare: true, term: true, watch: true, snap: true,
		why: "per-agent roll-up of a run; model and token counts for a visible session are already in its snapshot"},
	{topic: "library.changed", view: true, bare: true, term: true, watch: true, snap: true,
		why: "an EMPTY payload — a bare refetch signal with no data field at all"},

	// ---- the sidecar / plugin control plane: host-only ----------------------
	// Every column false. No capability returns any of these, so no column can
	// be true without the topic having been re-decided.
	{topic: "plugin.log",
		why: "one VERBATIM unredacted line of a sidecar's stdout/stderr, whose environment carries WKS_SETTINGS with secret plugin settings in PLAINTEXT — a view token was observed receiving GITHUB_TOKEN=ghp_…"},
	{topic: "sidecar.running",
		why: "statusData{Name,State,PID,Err}: a host process id and raw spawn/exec error text carrying absolute paths and argv"},
	{topic: "plugin.loaded",
		why: "the whole Manifest — install argv and source, the sidecar's server command/args, and EVERY declared filesystem path scope, i.e. a map of what each sidecar may reach. A reader of this topic knows where to aim the next chain"},
	{topic: "plugin.unloaded",
		why: "the other edge of plugin.loaded — which sidecar stopped being supervised"},
	{topic: "plugin.settings.changed",
		why: "secrets are redacted but every non-secret value (endpoints, org/repo names, absolute paths) is verbatim, and the equivalent READ (/plugins/settings) is guarded to the host token"},
	{topic: "plugin.sandboxed",
		why: "which OS confinement mechanism a sidecar got — the confinement inventory of the host"},
	{topic: "plugin.sandbox.refused",
		why: "why a sandbox could not be applied — the negative half of that inventory"},
	{topic: "plugin.unsandboxed",
		why: "announces which sidecars run with NO filesystem confinement, and why: it names the process to attack and states that nothing will contain it"},
	{topic: "plugin.install.progress",
		why: "echoes body.URL VERBATIM from the operator-guarded POST /plugins/install, unnormalized"},

	// ---- the default -------------------------------------------------------
	// Not in the registry, and that is the point: a scoped user token is refused
	// what nobody classified, while a plugin's manifest is still a real answer
	// for a topic that is not host state (example.clock.tick, command.*).
	{topic: unclassifiedProbe, bare: true, term: true, watch: true, snap: true,
		why: "unclassified: refused to a scoped user token (fail closed), still manifest-gated for a plugin — plugins must keep their own event model"},
}

func (r topicDelivery) want(credential string) bool {
	switch credential {
	case "view":
		return r.view
	case "bare":
		return r.bare
	case "term":
		return r.term
	case "watch":
		return r.watch
	case "snap":
		return r.snap
	case "op":
		return true
	}
	return false
}

func TestEachTopicReachesExactlyTheCredentialsEntitledToIt(t *testing.T) {
	url, srv := scopedServer(t)
	all := capspec.EventGrants{Consumes: []string{"*"}}
	srv.RegisterPluginToken("plug-bare", "bare.plugin", nil, all)
	srv.RegisterPluginToken("plug-term", "term.plugin",
		[]capspec.Grant{{Method: "sessions.attachTerminal"}}, all)
	srv.RegisterPluginToken("plug-watch", "watch.plugin",
		[]capspec.Grant{{Method: "fs.watch"}}, all)
	srv.RegisterPluginToken("plug-snap", "snap.plugin",
		[]capspec.Grant{{Method: "sessions.snapshot"}}, all)

	credentials := []struct{ key, token, label string }{
		{"view", "tok-view", "a scoped `view` token"},
		{"bare", "plug-bare", "a plugin consuming \"*\" that holds NO capability"},
		{"term", "plug-term", "a plugin holding ONLY sessions.attachTerminal"},
		{"watch", "plug-watch", "a plugin holding ONLY fs.watch"},
		{"snap", "plug-snap", "a plugin holding ONLY sessions.snapshot"},
		{"op", "tok-operator", "the operator tier"},
	}

	conns := map[string]*client{}
	for _, c := range credentials {
		cl := dialClientToken(t, url, c.token)
		cl.send(Frame{Op: "subscribe", Topics: []string{"*"}})
		cl.readUntil("subscribed")
		conns[c.key] = cl
	}
	if s := conns["view"].hello.Scope; s != "view" {
		t.Fatalf("hello scope = %q, want view — the matrix is not talking to the tier it claims", s)
	}

	host := dialClientToken(t, url, "host-secret")
	for _, row := range topicDeliveryKey {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: row.topic,
			Data: json.RawMessage(`{"secret":"MATRIX"}`)}})
	}
	// Published LAST and open by decision, so every credential ends its drain on
	// it. If a mutation makes it undeliverable the drain falls out on the timeout
	// and the missing-delivery assertions below name it.
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: sentinelTopic,
		Data: json.RawMessage(`{"sentinel":true}`)}})

	for _, c := range credentials {
		got := drainEvents(conns[c.key], sentinelTopic)
		for _, row := range topicDeliveryKey {
			switch want := row.want(c.key); {
			case want && !got[row.topic]:
				t.Errorf("%s did NOT receive %q, and must: %s", c.label, row.topic, row.why)
			case !want && got[row.topic]:
				t.Errorf("LEAK: %s received %q — %s", c.label, row.topic, row.why)
			}
		}
	}

	// The key above is hand-written, so a NEW registry row would simply not be
	// asserted on. This is the forcing function for that: every classified topic
	// must have an expected delivery per credential, decided from its payload.
	covered := map[string]bool{}
	for _, row := range topicDeliveryKey {
		spec, ok := capspec.EventTopicSpec(row.topic)
		if !ok {
			if row.topic != unclassifiedProbe {
				t.Errorf("the answer key probes %q, which the registry no longer classifies — the row was renamed and its guard now protects nothing", row.topic)
			}
			continue
		}
		covered[spec.Pattern] = true
	}
	for _, r := range capspec.EventTopics() {
		if !covered[r.Pattern] {
			t.Errorf("the registry classifies %q and the answer key in eventplane_test.go says nothing about it. Add a topicDeliveryKey row stating, from the PAYLOAD, which credentials may receive it — the matrix this replaced derived that from the disposition itself and so could never disagree with it.", r.Pattern)
		}
	}
	if len(covered) < 18 {
		t.Fatalf("the key pins only %d registry patterns — it shrank, and a shrinking answer key is how a matrix stops asserting", len(covered))
	}
}

const sentinelTopic = "agent.state_changed"

// drainEvents reads until the sentinel, or until the socket goes quiet. The
// timeout is the honest half: a credential that receives NOTHING must produce a
// result rather than a hang, because "delivered nothing" is a legitimate (and
// for host-only topics, required) outcome.
func drainEvents(c *client, sentinel string) map[string]bool {
	got := map[string]bool{}
	for {
		f, ok := c.tryReadUntil("event", "event", time.Second)
		if !ok {
			return got
		}
		if f.Event == nil {
			continue
		}
		got[f.Event.Type] = true
		// The sentinel is identified by its PAYLOAD, not its type: the type is
		// itself one of the probed rows, and stopping on the probe left every
		// later topic unread and every later assertion vacuously "not delivered".
		if f.Event.Type == sentinel && strings.Contains(string(f.Event.Data), `"sentinel"`) {
			return got
		}
	}
}

// The three proven pty.* escapes, each asserted by name, because they are
// siblings on ONE stream and the previous round guarded one of them.
func TestThePtyFamilyIsGuardedAsAFamily(t *testing.T) {
	url, _ := scopedServer(t)
	view := dialClientToken(t, url, "tok-view")
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	for _, typ := range []string{"pty.bytes.SECRET-42", "pty.exit", "pty.desync"} {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: typ,
			Data: json.RawMessage(`{"sessionId":"SECRET-42"}`)}})
	}
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.state_changed"}})

	f := view.readUntil("event")
	if f.Event == nil || f.Event.Type != "agent.state_changed" {
		t.Fatalf("a view token received %q — every pty.* topic is the output of sessions.attachTerminal, which this tier is refused", f.Event.Type)
	}

	// And the operator, who DOES hold the capability, still gets all three: a
	// guard that breaks the remote terminal is not a fix.
	op := dialClientToken(t, url, "tok-operator")
	op.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	op.readUntil("subscribed")
	for _, typ := range []string{"pty.bytes.SECRET-42", "pty.exit", "pty.desync"} {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: typ}})
		if f := op.readUntil("event"); f.Event == nil || f.Event.Type != typ {
			t.Fatalf("operator did not receive %q", typ)
		}
	}
}

// PROVEN LEAK, kept as a test. The delivery guard was a DELIVERY filter, not a
// subscription filter: broker.Publish matched pty.bytes.<id> and ENQUEUED it
// into the denied token's channel, and only the writer goroutine dropped it —
// after the channel had overflowed and sub.noteDrop had recorded the topic. That
// bookkeeping was then published back to the same connection as pty.desync,
// naming the sessionId of a stream it may not consume.
//
// The buffer size is the only thing this test changes; the real drop condition
// is an ordinary slow client (a phone on cellular).
func TestARefusedStreamLeavesNoDesyncTrail(t *testing.T) {
	srv := NewServer(broker.NewWithBuffer(1))
	url := serveTest(t, srv)
	srv.SetToken("host-secret")
	installScopedTiers(srv)

	view := dialClientToken(t, url, "tok-view")
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	for i := 0; i < 300; i++ {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-SESSION-42",
			Data: json.RawMessage(`"c2VjcmV0"`)}})
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.state_changed"}})
	}
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.snapshot",
		Data: json.RawMessage(`{"sentinel":true}`)}})

	for i := 0; i < 700; i++ {
		// Bounded: under a buffer of 1 the sentinel itself may be dropped, and a
		// test that then blocked forever would be a hang rather than a result.
		f, ok := view.tryReadUntil("event", "event", 500*time.Millisecond)
		if !ok {
			break
		}
		if f.Event == nil {
			continue
		}
		if strings.HasPrefix(f.Event.Type, "pty.") {
			t.Fatalf("LEAK: a view token received %s data=%s — the identity of a stream this connection was refused, arriving as the refused stream's own drop bookkeeping",
				f.Event.Type, f.Event.Data)
		}
		if f.Event.Type == "agent.snapshot" && strings.Contains(string(f.Event.Data), "sentinel") {
			return
		}
	}
	// Reaching here means the sentinel was itself dropped by the overflow, which
	// is legitimate under a buffer of 1 — the assertion above is the one that
	// matters and it ran on every frame that did arrive.
}

// FINDING 1, critical. eventTopicGuards was consulted on ONE arm of mayConsume
// and skipped on the other. A plugin with ZERO capability grants and
// `consumes: ["pty.bytes.*","fs.changed"]` was refused sessions.attachTerminal
// and fs.watch on the call plane and handed both capabilities' entire output on
// the event plane — the same crossing round 6 closed for scoped user tokens,
// still open for the plugin credential class. The install-consent dialog, the
// exemption's stated justification, rendered both consume lines at
// severity=normal.
func TestPluginConsumesCannotOutrunItsCapabilityGrants(t *testing.T) {
	url, srv := scopedServer(t, "sessions.attachTerminal", "fs.watch")
	srv.RegisterPluginToken("plug-tok", "evil.plugin", nil, capspec.EventGrants{
		Consumes: []string{"pty.bytes.*", "fs.changed", "plugin.log", "marker.done"},
	})

	plug := dialClientToken(t, url, "plug-tok")
	plug.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	plug.readUntil("subscribed")

	// The call plane refuses both, asserted rather than assumed.
	for _, m := range []string{"sessions.attachTerminal", "fs.watch"} {
		plug.send(Frame{Op: "call", ID: m, Method: m, Params: json.RawMessage(`{"sessionId":"s1","path":"/tmp"}`)})
		if e := plug.readUntil("error"); !strings.Contains(e.Error, "not authorized") {
			t.Fatalf("%s error = %q, want the plugin capability refusal", m, e.Error)
		}
	}

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-42",
		Data: json.RawMessage(`"JCBjYXQgfi8uYXdzL2NyZWRlbnRpYWxz"`)}})
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "fs.changed",
		Data: json.RawMessage(`{"path":"/home/u/.ssh/id_ed25519","eventType":"change"}`)}})
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "plugin.log",
		Data: json.RawMessage(`{"line":"GITHUB_TOKEN=ghp_deadbeef"}`)}})
	// A topic the manifest declares that NOBODY classifies: the plugin must
	// still receive it, or the fix has broken the plugin event model.
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "marker.done"}})

	f := plug.readUntil("event")
	if f.Event == nil {
		t.Fatal("no event delivered")
	}
	if f.Event.Type != "marker.done" {
		t.Fatalf("a plugin holding NO capabilities received %q data=%s — its manifest string granted it the whole output of a capability the call plane had just refused it",
			f.Event.Type, f.Event.Data)
	}

	// And the mirror: a plugin that DOES hold sessions.attachTerminal receives
	// the stream, so the manifest stays a filter rather than becoming a blanket.
	srv.RegisterPluginToken("good-tok", "good.plugin", []capspec.Grant{{Method: "sessions.attachTerminal"}},
		capspec.EventGrants{Consumes: []string{"pty.bytes.*"}})
	good := dialClientToken(t, url, "good-tok")
	good.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	good.readUntil("subscribed")
	host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-42"}})
	if f := good.readUntil("event"); f.Event == nil || f.Event.Type != "pty.bytes.SECRET-42" {
		t.Fatalf("a plugin granted sessions.attachTerminal did not receive the stream: %+v", f.Event)
	}
}

// FINDINGS 3 and 4, the publish direction. A plugin whose manifest declares
// `emits: ["layout.changed"]` or `["agent.snapshot"]` was publishing HOST STATE:
// a layout document every client adopts verbatim (carrying the four
// spawn-escalation fields layout.set scrubs, plus a publisher-chosen version
// that wins every later comparison), and the snapshot internal/push turns into
// the phone's "needs you" lock-screen notification.
func TestNobodyButTheHostMayPublishHostState(t *testing.T) {
	url, srv := scopedServer(t)
	srv.RegisterPluginToken("plug-tok", "evil.plugin", nil, capspec.EventGrants{
		Emits: []string{"layout.changed", "agent.snapshot", "plugin.settings.changed", "myplugin.tick"},
	})

	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	host.readUntil("subscribed")

	plug := dialClientToken(t, url, "plug-tok")
	for _, typ := range []string{"layout.changed", "agent.snapshot", "plugin.settings.changed"} {
		plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: typ,
			Data: json.RawMessage(`{"forged":true}`)}})
		e := plug.readUntil("error")
		if !strings.Contains(e.Error, "not authorized") {
			t.Fatalf("publishing %q as a plugin returned %q, want a refusal — every one of these topics is read by a trusted in-hub consumer as authoritative host state", typ, e.Error)
		}
	}
	// Its OWN topic still works: plugins emit their own events, and classifying
	// host topics must not take that away.
	plug.send(Frame{Op: "publish", Event: &event.Envelope{Type: "myplugin.tick"}})
	if f := host.readUntil("event"); f.Event == nil || f.Event.Type != "myplugin.tick" {
		t.Fatalf("a plugin's own declared topic did not publish: %+v", f.Event)
	}

	// A scoped user token cannot publish at all, in either direction.
	viewc := dialClientToken(t, url, "tok-view")
	viewc.send(Frame{Op: "publish", Event: &event.Envelope{Type: "agent.snapshot"}})
	if e := viewc.readUntil("error"); !strings.Contains(e.Error, "not authorized") {
		t.Fatalf("a view token published agent.snapshot: %q", e.Error)
	}
}

// installScopedTiers wires the REAL authtoken tiers into a server, so every
// test in this package that talks about "view" or "triage" is talking about the
// tier the product actually mints rather than a list a test invented.
func installScopedTiers(srv *Server) {
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		switch tok {
		case "tok-view":
			return ScopedIdent{Scope: "view", Methods: authtoken.ScopeView.Methods()}, true
		case "tok-triage":
			return ScopedIdent{Scope: "triage", Methods: authtoken.ScopeTriage.Methods()}, true
		case "tok-operator":
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		}
		return ScopedIdent{}, false
	})
}

// serveTest exposes a server whose broker the caller built (the desync test
// needs a buffer of 1), which rpcServerWith cannot do.
func serveTest(t *testing.T, srv *Server) string {
	t.Helper()
	hs := httptest.NewServer(srv.Handler())
	t.Cleanup(hs.Close)
	return hs.URL
}

// The bus must actually INSTALL the admission filter. The broker's own test
// pins the filter's behaviour, and the desync test pins the wire, but both stay
// green with `broker.Subscribe(nil)` back in place: the writer-side mayConsume
// check hides the wiring by refusing the same events one step later. What it
// cannot hide is the COST — an event that reached the channel took a slot and,
// past capacity, was recorded as a drop, and that record is what escaped as
// pty.desync.
//
// So this asserts the negative directly: a refused topic must leave the denied
// connection's drop count at zero, however much of it is published.
func TestTheBusFiltersRefusedTopicsBeforeTheyReachTheChannel(t *testing.T) {
	b := broker.NewWithBuffer(1)
	srv := NewServer(b)
	url := serveTest(t, srv)
	srv.SetToken("host-secret")
	installScopedTiers(srv)

	view := dialClientToken(t, url, "tok-view")
	view.send(Frame{Op: "subscribe", Topics: []string{"*"}})
	view.readUntil("subscribed")

	host := dialClientToken(t, url, "host-secret")
	// Enough that an unfiltered path cannot keep up: the reader is idle, so the
	// socket backs up, the writer goroutine blocks, and a channel of capacity 1
	// starts discarding — which is exactly the production condition (a phone on
	// cellular) the desync signal exists for.
	for i := 0; i < 20000; i++ {
		host.send(Frame{Op: "publish", Event: &event.Envelope{Type: "pty.bytes.SECRET-42",
			Data: json.RawMessage(`"c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0"`)}})
	}
	// Let the fan-out settle: publishes are async relative to this goroutine.
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) && b.SubscriberCount() > 0 {
		if n := b.DroppedTotal(); n > 0 {
			t.Fatalf("a topic the view tier may not consume was enqueued and then discarded %d times. It cost this connection buffer capacity it was never entitled to, and each discard on a stream topic is remembered BY NAME — which is the record that escaped as a pty.desync event naming the sessionId.", n)
		}
		time.Sleep(50 * time.Millisecond)
	}
	if n := b.DroppedTotal(); n != 0 {
		t.Fatalf("drop count = %d, want 0", n)
	}
	view.ws.CloseNow()
}
