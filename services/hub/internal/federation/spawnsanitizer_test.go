package federation

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

// THE SPAWN SANITIZER, ACROSS A LIVE FEDERATED HOP.
//
// internal/bus already proves the clamp against a FAKE forwarder: a stub that
// records the params and never speaks to anything. That proves the LOCAL side
// applies the sanitizer before the hop, which is half the claim. The other half
// — the peer re-judging what arrives, against its OWN ceiling and its OWN view
// of the link's credential — has no stand-in, because the party doing the
// judging is a second router.
//
// So this test runs two real bus servers over two real websocket links with a
// real federation.Manager between them. It is in-process rather than two OS
// processes, and deliberately: the thing under test is the ROUTER's behaviour on
// each side, both routers here are the real bus.Server on real transport, and a
// process boundary would add supervision and port allocation without adding a
// single line of the code being judged.

type spawnHubs struct {
	localURL string
	// peerSaw receives the params each agents.spawn the PEER's provider was
	// actually handed — the far side of the hop, which is the whole point.
	peerSaw chan json.RawMessage
}

// twoHubs stands up a peer hub (host token, scoped link token, a directory
// ceiling and an agents.spawn provider) and a local hub federated to it.
func twoHubs(t *testing.T, ctx context.Context, linkYolo bool) spawnHubs {
	t.Helper()

	// ── THE PEER ────────────────────────────────────────────────────────────
	peerBus := bus.NewServer(broker.New())
	peerBus.SetToken("peer-host-token")
	peerBus.SetScopedTokenLookup(func(tok string) (bus.ScopedIdent, bool) {
		if tok == "link-token" {
			return bus.ScopedIdent{
				Scope:       "operator",
				Methods:     authtoken.ScopeOperator.Methods(),
				YoloAllowed: linkYolo,
			}, true
		}
		return bus.ScopedIdent{}, false
	})
	// The peer's OWN ceiling. The local hub has none wired, so anything clamped
	// below was clamped by the machine the work will run on.
	peerBus.SetSpawnCeiling(func(req bus.SpawnCeilingRequest) bus.SpawnCeilingVerdict {
		v := bus.SpawnCeilingVerdict{Key: "default", MaxCapability: "balanced", MaxToolScope: "view"}
		if req.Capability != "" && req.Capability != "balanced" {
			v.CapabilityRefused, v.Capability = true, "balanced"
			v.Provider, v.Model, v.Effort = "codex", "gpt-5.6-terra", "high"
			v.Because = []string{"the peer caps this directory at balanced"}
		}
		if req.ToolScope != "" && req.ToolScope != "view" {
			v.ToolScopeRefused, v.ToolScope = true, "view"
			v.Because = append(v.Because, "the peer caps this directory at the view tier")
		}
		// The peer's own FRESHNESS rule, the non-ceiling half of the same
		// resolver. It refuses rather than clamps, so a spawn that trips it
		// never reaches the peer's provider at all.
		if req.Resuming && req.Role == "reviewer" {
			v.ResumeRefused, v.FreshCapability = true, "reviewer"
			v.Because = append(v.Because,
				"the peer marks capability reviewer `fresh: true`, so session "+req.ResumeSessionID+" may not be inherited")
		}
		return v
	}, nil)
	peerSrv := httptest.NewServer(peerBus.Handler())
	t.Cleanup(peerSrv.Close)
	peerURL := strings.Replace(peerSrv.URL, "http", "ws", 1) + "/bus"

	saw := make(chan json.RawMessage, 8)
	prov := dial(t, ctx, peerURL+"?token=peer-host-token")
	t.Cleanup(func() { _ = prov.CloseNow() })
	send(t, ctx, prov, wsFrame{Op: "register", Methods: []string{"agents.spawn"}})
	go func() {
		for {
			_, data, err := prov.Read(ctx)
			if err != nil {
				return
			}
			var f wsFrame
			if json.Unmarshal(data, &f) != nil || f.Op != "call" {
				continue
			}
			saw <- f.Params
			send(t, ctx, prov, wsFrame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"sessionId":"peer-1"}`)})
		}
	}()

	// ── THE LOCAL HUB ───────────────────────────────────────────────────────
	localBroker := broker.New()
	localBus := bus.NewServer(localBroker)
	localBus.SetToken("local-host-token")
	localBus.SetScopedTokenLookup(func(tok string) (bus.ScopedIdent, bool) {
		if tok == "local-operator" {
			return bus.ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		}
		return bus.ScopedIdent{}, false
	})
	fed, err := New(localBroker, []Peer{{Name: "work", URL: peerURL, Token: "link-token"}})
	if err != nil {
		t.Fatal(err)
	}
	localBus.SetFederation(fed)
	go fed.Run(ctx)
	localSrv := httptest.NewServer(localBus.Handler())
	t.Cleanup(localSrv.Close)

	return spawnHubs{
		localURL: strings.Replace(localSrv.URL, "http", "ws", 1) + "/bus?token=local-operator",
		peerSaw:  saw,
	}
}

// spawnAcross sends one hub:work/agents.spawn from the local hub and returns the
// reply frame. It waits for the link before calling — the federation manager
// dials with backoff, so an immediate call would fail on "unknown peer" rather
// than on anything this test is about.
func spawnAcross(t *testing.T, ctx context.Context, h spawnHubs, params string) wsFrame {
	t.Helper()
	c := dial(t, ctx, h.localURL)
	defer c.CloseNow()

	deadline := time.Now().Add(15 * time.Second)
	for attempt := 0; ; attempt++ {
		send(t, ctx, c, wsFrame{Op: "call", ID: "s1", Method: "hub:work/agents.spawn",
			Params: json.RawMessage(params)})
		f := awaitReply(t, ctx, c, "s1")
		if !strings.Contains(f.Error, "not connected") && !strings.Contains(f.Error, "unknown federation peer") {
			return f
		}
		if time.Now().After(deadline) {
			t.Fatalf("the federation link never came up: %q", f.Error)
		}
		time.Sleep(200 * time.Millisecond)
	}
}

func awaitReply(t *testing.T, ctx context.Context, c *websocket.Conn, id string) wsFrame {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		_, data, err := c.Read(ctx)
		if err != nil {
			t.Fatalf("read: %v", err)
		}
		var f wsFrame
		if json.Unmarshal(data, &f) != nil {
			continue
		}
		if f.ID == id && (f.Op == "result" || f.Op == "error") {
			return f
		}
	}
	t.Fatalf("no reply to %s", id)
	return wsFrame{}
}

func nothingReached(t *testing.T, h spawnHubs) {
	t.Helper()
	select {
	case raw := <-h.peerSaw:
		t.Fatalf("the peer's provider was handed a spawn it should never have seen: %s", raw)
	case <-time.After(500 * time.Millisecond):
	}
}

func peerParams(t *testing.T, h spawnHubs) map[string]any {
	t.Helper()
	select {
	case raw := <-h.peerSaw:
		var m map[string]any
		if err := json.Unmarshal(raw, &m); err != nil {
			t.Fatalf("peer params did not decode (%v): %s", err, raw)
		}
		return m
	case <-time.After(10 * time.Second):
		t.Fatal("the peer's provider was never called")
		return nil
	}
}

// 1. THE ALIAS REFUSAL BINDS ON THE HOP. A capitalized authority key must not be
// repaired into a peer's provider any more than into a local one.
func TestAnAliasedAuthorityKeyNeverCrossesToALivePeer(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	h := twoHubs(t, ctx, false)

	f := spawnAcross(t, ctx, h, `{"cwd":"/x","YoloGranted":true,"skipPermissions":true}`)
	if f.Op != "error" {
		t.Fatalf("an aliased authority key was accepted for a federated spawn: %+v", f)
	}
	if !strings.Contains(f.Error, "yoloGranted") {
		t.Errorf("the refusal does not name the canonical spelling: %q", f.Error)
	}
	nothingReached(t, h)
}

// 2. THE PEER'S OWN CEILING CLAMPS WHAT ARRIVES. The local hub has no ceiling
// wired at all, so everything asserted here was decided by the second router —
// which is the half a fake forwarder cannot show.
func TestThePeersOwnCeilingClampsAFederatedSpawn(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	h := twoHubs(t, ctx, false)

	f := spawnAcross(t, ctx, h,
		`{"cwd":"/x","capability":"frontier_plus","model":"fable","effort":"max","toolScope":"operator"}`)
	if f.Op != "result" {
		t.Fatalf("the federated spawn failed: %+v", f)
	}
	got := peerParams(t, h)

	if got["capability"] != "balanced" {
		t.Errorf("the peer's capability ceiling did not bind: %v", got)
	}
	if got["model"] != "gpt-5.6-terra" {
		t.Errorf("the peer did not replace the refused model with its own routed one: %v", got)
	}
	if got["toolScope"] != "view" {
		t.Errorf("the peer's tool-tier ceiling did not bind: %v", got)
	}
}

// 3. A LINK WITHOUT THE FULL-ACCESS GRANT CANNOT CARRY ONE. Neither hub stamps
// `yoloGranted`: the local caller is not full-access, and a federation link
// inherits no host trust from having been authenticated.
func TestAFederatedSpawnGetsNoFullAccessStampFromAnUngrantedLink(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	h := twoHubs(t, ctx, false)

	if f := spawnAcross(t, ctx, h, `{"cwd":"/x","yoloGranted":true,"skipPermissions":true}`); f.Op != "result" {
		t.Fatalf("the federated spawn failed: %+v", f)
	}
	got := peerParams(t, h)
	if _, has := got["yoloGranted"]; has {
		t.Errorf("a spoofed full-access stamp survived two routers: %v", got)
	}
	// The REQUEST rides through untouched — the stamp is about the caller, not
	// the ask, and the provider clamps an unstamped request itself.
	if got["skipPermissions"] != true {
		t.Errorf("the skipPermissions REQUEST was altered rather than left to the provider's clamp: %v", got)
	}
}

// 4. …and a link the far hub DID trust with full access gets the stamp, from the
// peer. Otherwise the test above would pass for a hub that simply never stamps.
func TestAFullAccessGrantedLinkIsStampedByThePeer(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	h := twoHubs(t, ctx, true)

	if f := spawnAcross(t, ctx, h, `{"cwd":"/x","skipPermissions":true}`); f.Op != "result" {
		t.Fatalf("the federated spawn failed: %+v", f)
	}
	got := peerParams(t, h)
	if got["yoloGranted"] != true {
		t.Errorf("a link minted WITH the full-access grant was not stamped by the peer: %v", got)
	}
}

// 5. THE PEER'S OWN FRESHNESS RULE REFUSES A RESUME. The local hub has no
// routing layer wired, so this refusal was reached entirely by the second
// router — and it is the arm that a fake forwarder cannot show, because the
// party that must say no is the machine the reviewer would actually run on.
//
// A reviewer that could inherit the implementer's conversation by naming
// `hub:work/agents.spawn` would be one hop away from walking around the whole
// guarantee, which is exactly why the enforcement lives in the sanitizer table
// both call paths share rather than in either one of them.
func TestThePeerRefusesAFederatedResumeForAFreshRole(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	h := twoHubs(t, ctx, false)

	f := spawnAcross(t, ctx, h, `{"cwd":"/x","role":"reviewer","resumeSessionId":"implementer-sess-1"}`)
	if f.Op != "error" {
		t.Fatalf("a federated reviewer inherited a session across the hop: %+v", f)
	}
	if !strings.Contains(f.Error, "implementer-sess-1") || !strings.Contains(f.Error, "fresh") {
		t.Errorf("the peer's refusal did not come back to the local caller with its reason: %q", f.Error)
	}
	nothingReached(t, h)
}

// …and the same hop with no fresh role attached still carries its resume, so the
// case above measures the rule rather than a federated spawn that simply cannot
// resume anything.
func TestAnOrdinaryFederatedResumeStillReachesThePeer(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	h := twoHubs(t, ctx, false)

	if f := spawnAcross(t, ctx, h, `{"cwd":"/x","role":"implementer","resumeSessionId":"sess-7"}`); f.Op != "result" {
		t.Fatalf("an ordinary federated resume was refused: %+v", f)
	}
	if got := peerParams(t, h); got["resumeSessionId"] != "sess-7" {
		t.Errorf("an ordinary resume lost its session id across the hop: %v", got)
	}
}
