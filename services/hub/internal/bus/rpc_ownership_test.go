package bus

// SINGLE-OWNER REGISTRATION and what a caller sees when nobody provides a
// method. Two silences lived here: an ack that asserted an ownership the router
// would never honour (a hub-local handler shadows every remote provider), and a
// no-provider call that told only its caller — no hub log line, no bus event,
// and a /health that exposed a count nobody can interpret.
//
// Driven over real connections against the real Handler(), because the claim is
// about what a CLIENT is told.

import (
	"bytes"
	"encoding/json"
	"log"
	"net/http"
	"strings"
	"testing"
	"time"
)

func captureBusLog(t *testing.T) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	prevOut, prevFlags := log.Writer(), log.Flags()
	log.SetOutput(&buf)
	log.SetFlags(0)
	t.Cleanup(func() {
		log.SetOutput(prevOut)
		log.SetFlags(prevFlags)
	})
	return &buf
}

// The `registered` ack is the desktop's ONLY drift detector (hubClient.ts
// derives its whole withheld report from it), so it must never claim a method
// the router will not route to this provider. A hub-local handler takes
// precedence in call(), so accepting one here asserted an ownership that could
// never be honoured — and in the SAFE direction, which is why nobody noticed.
func TestRegisterAckWithholdsAMethodAHubLocalHandlerShadows(t *testing.T) {
	url := newTestServerWith(t, func(s *Server) {
		s.RegisterLocal("layout.get", func(json.RawMessage) (any, error) { return "hub", nil })
		s.RegisterLocalIdent("push.subscribe", func(CallerIdentity, json.RawMessage) (any, error) { return "hub", nil })
	})
	provider := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"layout.get", "push.subscribe", "git.status"}})
	ack := provider.readUntil("registered")

	got := strings.Join(ack.Methods, ",")
	if got != "git.status" {
		t.Fatalf("ack accepted %q — it claims methods the hub answers in-process, so the provider believes it owns something the router never asks it for (and the desktop's drift report is silently wrong)", got)
	}
}

// The registered provider really is never called for the shadowed method: the
// ack above is not merely cosmetic.
func TestAHubLocalHandlerStillAnswersAShadowedMethod(t *testing.T) {
	url := newTestServerWith(t, func(s *Server) {
		s.RegisterLocal("layout.get", func(json.RawMessage) (any, error) { return "hub", nil })
	})
	provider := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"layout.get"}})
	provider.readUntil("registered")

	caller := dialClient(t, url)
	caller.send(Frame{Op: "call", ID: "1", Method: "layout.get"})
	res := caller.readUntil("result")
	if string(res.Result) != `"hub"` {
		t.Fatalf("layout.get answered %s, want the hub's own handler", res.Result)
	}
}

func TestACallWithNoProviderIsSaidOutLoudOncePerMethod(t *testing.T) {
	buf := captureBusLog(t)
	url := rpcServer(t)
	caller := dialClient(t, url)

	for i := 0; i < 3; i++ {
		caller.send(Frame{Op: "call", ID: "1", Method: "config.save"})
		caller.readUntil("error")
	}
	caller.send(Frame{Op: "call", ID: "2", Method: "library.list"})
	caller.readUntil("error")

	out := buf.String()
	if !strings.Contains(out, `NO PROVIDER for "config.save"`) {
		t.Fatalf("a no-provider call left no trace anywhere but the caller's own error frame — an entire capability plane can die in silence.\nlog was: %q", out)
	}
	if !strings.Contains(out, `NO PROVIDER for "library.list"`) {
		t.Fatalf("only the first missing method was reported.\nlog was: %q", out)
	}
	if n := strings.Count(out, `NO PROVIDER for "config.save"`); n != 1 {
		t.Fatalf("logged %d times for one method — a retrying client would drown the log; want once", n)
	}
}

// A provider arriving and then going away is a NEW outage, worth saying again.
func TestNoProviderIsSaidAgainAfterAProviderComesAndGoes(t *testing.T) {
	buf := captureBusLog(t)
	url := rpcServer(t)
	caller := dialClient(t, url)

	caller.send(Frame{Op: "call", ID: "1", Method: "config.save"})
	caller.readUntil("error")

	provider := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"config.save"}})
	provider.readUntil("registered")
	provider.ws.CloseNow()

	// The drop is processed on the server's read loop; retry until ownership is
	// released and the call comes back as "no provider".
	released := false
	for i := 0; i < 100 && !released; i++ {
		caller.send(Frame{Op: "call", ID: "2", Method: "config.save"})
		f := caller.readUntil("error")
		released = strings.Contains(f.Error, "no provider")
		if !released {
			time.Sleep(20 * time.Millisecond)
		}
	}
	if !released {
		t.Fatal("provider ownership was never released after the connection dropped")
	}

	if n := strings.Count(buf.String(), `NO PROVIDER for "config.save"`); n != 2 {
		t.Fatalf("a second, separate outage was reported %d times, want 2", n)
	}
}

// A client cannot render "this plane is dead" if it cannot ask what is provided.
// /health used to expose only a COUNT, which mixes hub-local handlers with real
// providers and which nothing compares to any expectation.
func TestProvidedMethodsNamesEverythingTheBusCanAnswer(t *testing.T) {
	rt := newRouter()
	rt.registerLocal("layout.get", func(json.RawMessage) (any, error) { return nil, nil })
	rt.registerLocalIdent("push.subscribe", func(CallerIdentity, json.RawMessage) (any, error) { return nil, nil })
	cn := &conn{trusted: true}
	rt.addConn(cn)
	rt.register(cn, []string{"config.get", "agents.list"})

	got := strings.Join(rt.providedMethods(), ",")
	want := "agents.list,config.get,layout.get,push.subscribe"
	if got != want {
		t.Fatalf("providedMethods() = %q, want %q (sorted union of remote providers + hub-local handlers)", got, want)
	}
}

func TestHealthNamesTheMethodsNotJustTheCount(t *testing.T) {
	url := newTestServerWith(t, func(s *Server) {
		s.RegisterLocal("layout.get", func(json.RawMessage) (any, error) { return nil, nil })
	})
	provider := dialClient(t, url)
	provider.send(Frame{Op: "register", Methods: []string{"config.get"}})
	provider.readUntil("registered")

	res, err := http.Get(url + "/health")
	if err != nil {
		t.Fatal(err)
	}
	defer res.Body.Close()
	var body map[string]any
	if err := json.NewDecoder(res.Body).Decode(&body); err != nil {
		t.Fatal(err)
	}
	names, _ := body["methodNames"].([]any)
	var got []string
	for _, n := range names {
		got = append(got, n.(string))
	}
	joined := strings.Join(got, ",")
	if !strings.Contains(joined, "config.get") || !strings.Contains(joined, "layout.get") {
		t.Fatalf("/health does not name what the bus provides (%q) — no client can tell a live plane from a dead one", joined)
	}
}
