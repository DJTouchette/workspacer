package bus

import (
	"encoding/json"
	"net/http"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// scopedServer builds a bus with a host token and the real authtoken tiers
// wired through SetScopedTokenLookup, plus a trusted provider answering every
// registered method — so tests exercise the genuine dispatch path end to end.
func scopedServer(t *testing.T, methods ...string) (url string, srv *Server) {
	t.Helper()
	url, srv = rpcServerWith(t)
	srv.SetToken("host-secret")
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
	if len(methods) > 0 {
		provider := dialClientToken(t, url, "host-secret")
		provider.send(Frame{Op: "register", Methods: methods})
		provider.readUntil("registered")
		go func() {
			for {
				f, ok := provider.tryRead("call")
				if !ok {
					return
				}
				provider.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
			}
		}()
	}
	return url, srv
}

// tryRead is readUntil without t.Fatal on connection close, for background
// provider loops that outlive the test body.
func (c *client) tryRead(op string) (Frame, bool) {
	for {
		_, data, err := c.ws.Read(c.t.Context())
		if err != nil {
			return Frame{}, false
		}
		var f Frame
		if err := json.Unmarshal(data, &f); err != nil {
			return Frame{}, false
		}
		if f.Op == op {
			return f, true
		}
	}
}

// TestScopedTokenAuthorizationMatrix is the tier × method truth table on the
// real routing path: every provided method either round-trips (allowed) or is
// refused by the bus with an error naming the token's scope — including a
// method no tier has ever heard of, which must fail closed for view/triage.
func TestScopedTokenAuthorizationMatrix(t *testing.T) {
	methods := []string{
		"agents.list", "sessions.snapshots", "sessions.transcript", // view surface
		"claude.approve", "claude.answer", "agents.sendMessage", "push.subscribe", // triage surface
		"agents.spawn", "terminals.create", "git.push", "config.save", // operator surface
		"future.unknownMethod", // registered on purpose: a deny must come from scope, not "no provider"
	}
	url, _ := scopedServer(t, methods...)

	cases := []struct {
		token  string
		scope  string
		method string
		allow  bool
	}{
		// view: read-only in, everything else out
		{"tok-view", "view", "agents.list", true},
		{"tok-view", "view", "sessions.snapshots", true},
		{"tok-view", "view", "sessions.transcript", true},
		{"tok-view", "view", "claude.approve", false},
		{"tok-view", "view", "agents.sendMessage", false},
		{"tok-view", "view", "push.subscribe", false},
		{"tok-view", "view", "agents.spawn", false},
		{"tok-view", "view", "git.push", false},
		{"tok-view", "view", "future.unknownMethod", false},
		// triage: view + acting on attention, still no spawn/terminals/git/admin
		{"tok-triage", "triage", "agents.list", true},
		{"tok-triage", "triage", "claude.approve", true},
		{"tok-triage", "triage", "claude.answer", true},
		{"tok-triage", "triage", "agents.sendMessage", true},
		{"tok-triage", "triage", "push.subscribe", true},
		{"tok-triage", "triage", "agents.spawn", false},
		{"tok-triage", "triage", "terminals.create", false},
		{"tok-triage", "triage", "git.push", false},
		{"tok-triage", "triage", "config.save", false},
		{"tok-triage", "triage", "future.unknownMethod", false},
		// operator: everything
		{"tok-operator", "operator", "agents.spawn", true},
		{"tok-operator", "operator", "git.push", true},
		{"tok-operator", "operator", "future.unknownMethod", true},
	}
	for _, c := range cases {
		t.Run(c.scope+"/"+c.method, func(t *testing.T) {
			caller := dialClientToken(t, url, c.token)
			caller.send(Frame{Op: "call", ID: "m1", Method: c.method})
			if c.allow {
				if r := caller.readUntil("result"); r.ID != "m1" {
					t.Fatalf("allowed call: got id %q, want m1", r.ID)
				}
				return
			}
			e := caller.readUntil("error")
			if e.ID != "m1" {
				t.Fatalf("denied call: got id %q, want m1", e.ID)
			}
			if !strings.Contains(e.Error, "not authorized") ||
				!strings.Contains(e.Error, `"`+c.scope+`"`) ||
				!strings.Contains(e.Error, c.method) {
				t.Fatalf("deny error = %q — must say not authorized and name the scope + method", e.Error)
			}
		})
	}
}

// TestLegacyHostTokenImpliesOperator pins backward compatibility: the persisted
// remote-token (the bus host token) has no scope record and keeps FULL access —
// it calls anything and still registers as a provider, exactly as before scoped
// tokens existed.
func TestLegacyHostTokenImpliesOperator(t *testing.T) {
	url, _ := scopedServer(t, "agents.spawn")

	legacy := dialClientToken(t, url, "host-secret")
	legacy.send(Frame{Op: "call", ID: "l1", Method: "agents.spawn"})
	if r := legacy.readUntil("result"); r.ID != "l1" {
		t.Fatalf("legacy token call: got id %q, want l1", r.ID)
	}
	legacy.send(Frame{Op: "register", Methods: []string{"desktop.extra"}})
	reg := legacy.readUntil("registered")
	if len(reg.Methods) != 1 || reg.Methods[0] != "desktop.extra" {
		t.Fatalf("legacy token register = %v, want [desktop.extra]", reg.Methods)
	}
}

// TestOperatorScopedTokenIsTrusted: an operator-tier minted token behaves like
// the host token on every verb, including provider registration.
func TestOperatorScopedTokenIsTrusted(t *testing.T) {
	url, _ := scopedServer(t)
	op := dialClientToken(t, url, "tok-operator")
	op.send(Frame{Op: "register", Methods: []string{"op.method"}})
	reg := op.readUntil("registered")
	if len(reg.Methods) != 1 || reg.Methods[0] != "op.method" {
		t.Fatalf("operator register = %v, want [op.method]", reg.Methods)
	}
}

// TestScopedTokenCannotProvideOrPublish: view/triage tokens are callers only.
// Registering yields an empty ack (methods withheld); publishing is refused
// with an error naming the scope.
func TestScopedTokenCannotProvideOrPublish(t *testing.T) {
	url, _ := scopedServer(t)
	cn := dialClientToken(t, url, "tok-triage")

	cn.send(Frame{Op: "register", Methods: []string{"claude.approve"}})
	reg := cn.readUntil("registered")
	if len(reg.Methods) != 0 {
		t.Fatalf("triage token registered %v — a scoped token must never become a provider (hijack risk)", reg.Methods)
	}

	ev := event.New("command.doSomething", "phone", nil)
	cn.send(Frame{Op: "publish", Event: &ev})
	e := cn.readUntil("error")
	if !strings.Contains(e.Error, `"triage"`) {
		t.Fatalf("publish deny = %q, want it to name the scope", e.Error)
	}
}

// TestScopedTokenReceivesEvents: event/stream subscriptions are part of the
// view tier — a view token subscribing to agent snapshots receives them.
func TestScopedTokenReceivesEvents(t *testing.T) {
	url, srv := scopedServer(t)
	viewer := dialClientToken(t, url, "tok-view")
	viewer.send(Frame{Op: "subscribe", Topics: []string{"agent.snapshot"}})
	viewer.readUntil("subscribed")

	srv.broker.Publish(event.New("agent.snapshot", "test", map[string]string{"s": "1"}))
	got := viewer.readUntil("event")
	if got.Event == nil || got.Event.Type != "agent.snapshot" {
		t.Fatalf("view token did not receive the snapshot event: %+v", got)
	}
}

// TestUnknownTokenStillRejectedWithLookupWired: wiring the scoped lookup must
// not widen the handshake — an unknown token is refused as before.
func TestUnknownTokenStillRejectedWithLookupWired(t *testing.T) {
	url, _ := scopedServer(t)
	wsURL := strings.Replace(url, "http://", "ws://", 1) + "/bus?token=bogus"
	resp, err := http.Get(strings.Replace(wsURL, "ws://", "http://", 1))
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("unknown token handshake = %d, want 401", resp.StatusCode)
	}
}

// TestAuthorizedHTTPWithScopedTokens: token-guarded HTTP routes (plugin admin,
// /remote, /app entry) are operator surface — the host token and an
// operator-scoped token pass; view/triage do not.
func TestAuthorizedHTTPWithScopedTokens(t *testing.T) {
	_, srv := scopedServer(t)
	req := func(tok string) *http.Request {
		r, _ := http.NewRequest(http.MethodGet, "/remote", nil)
		r.Header.Set("Authorization", "Bearer "+tok)
		return r
	}
	cases := []struct {
		token string
		want  bool
	}{
		{"host-secret", true},
		{"tok-operator", true},
		{"tok-triage", false},
		{"tok-view", false},
		{"bogus", false},
	}
	for _, c := range cases {
		if got := srv.Authorized(req(c.token)); got != c.want {
			t.Errorf("Authorized(%s) = %v, want %v", c.token, got, c.want)
		}
	}
}
