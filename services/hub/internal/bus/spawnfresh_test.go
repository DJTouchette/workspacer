package bus

import (
	"encoding/json"
	"strings"
	"testing"
)

// THE FRESHNESS REFUSAL, AT THE REAL DISPATCH POINT. Same discipline as
// spawnceiling_test.go next door: the whole path runs — handshake, tier check,
// sanitizeSpawnParams, forward — and the assertion is about what the PROVIDER
// was or was not handed.
//
// The resolver below is a STAND-IN for internal/routing, deliberately: the bus
// holds no matrix and must not learn one, so what is proven here is that the
// router refuses whatever it is told to refuse. internal/routing's fresh_test.go
// proves that what it is told is right.
func freshResolver(req SpawnCeilingRequest) SpawnCeilingVerdict {
	v := SpawnCeilingVerdict{Key: "default", MaxCapability: "frontier_plus", MaxToolScope: "operator"}
	if !req.Resuming {
		return v
	}
	if req.Role == "reviewer" || req.Capability == "deep_reviewer" {
		v.ResumeRefused, v.FreshCapability = true, "deep_reviewer"
		v.Because = []string{
			"this spawn asked to RESUME session " + req.ResumeSessionID +
				" while declaring review work, and routing.yaml marks capability deep_reviewer `fresh: true`",
		}
	}
	return v
}

// THE HEADLINE: a reviewer pointed at an existing session does not reach the
// provider, and the caller is told why in the answer rather than in a log line
// on a machine it cannot read.
func TestAFreshRoleCannotResumeASessionThroughTheSpawnGate(t *testing.T) {
	url, got, audits := ceilingServer(t, freshResolver)

	caller := dialClientToken(t, url, "tok-operator")
	caller.send(Frame{Op: "call", ID: "s1", Method: "agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp","role":"reviewer","resumeSessionId":"implementer-sess-1"}`)})
	f := caller.readUntil("error")
	if f.ID != "s1" {
		t.Fatalf("error frame id %q, want s1", f.ID)
	}
	if !strings.Contains(f.Error, "implementer-sess-1") || !strings.Contains(f.Error, "fresh") {
		t.Errorf("the refusal must name the session and the reason; got %q", f.Error)
	}
	select {
	case raw := <-got:
		t.Fatalf("a reviewer inherited the implementer's session anyway: %s", raw)
	default:
	}
	if len(*audits) != 1 {
		t.Fatalf("a refused spawn wrote %d audit rows, want 1", len(*audits))
	}
	rec := (*audits)[0]
	if !rec.Ceiling.ResumeRefused || rec.Ceiling.FreshCapability != "deep_reviewer" {
		t.Errorf("the audit row does not record the freshness refusal: %+v", rec.Ceiling)
	}
	if !rec.Ceiling.Refused() {
		t.Error("Refused() does not count a resume refusal, so an auditor reading it sees a clean spawn")
	}
}

// REFUSED, NOT SILENTLY DROPPED — the whole point of the arm. Dropping
// `resumeSessionId` would forward a spawn that starts a NEW session while the
// caller goes on believing it continued one, and that is the failure this shape
// was chosen to avoid. So nothing is forwarded at all.
func TestTheResumeIsRefusedRatherThanStrippedFromTheParams(t *testing.T) {
	url, got, _ := ceilingServer(t, freshResolver)

	caller := dialClientToken(t, url, "tok-operator")
	caller.send(Frame{Op: "call", ID: "s1", Method: "agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp","capability":"deep_reviewer","resumeSessionId":"sess-9"}`)})
	caller.readUntil("error")
	select {
	case raw := <-got:
		var m map[string]any
		_ = json.Unmarshal(raw, &m)
		if _, had := m["resumeSessionId"]; had {
			t.Fatalf("the spawn was forwarded WITH its resume: %s", raw)
		}
		t.Fatalf("the resume was stripped and the spawn forwarded as a NEW session the caller believes is a continuation: %s", raw)
	default:
	}
}

// A ROLE THAT IS NOT FRESH KEEPS ITS RESUME, and so does a spawn that declares
// nothing at all. Without this the refusal could be firing on every resume and
// the cases above would not notice.
func TestOrdinaryResumesStillReachTheProvider(t *testing.T) {
	url, got, _ := ceilingServer(t, freshResolver)

	for _, params := range []string{
		`{"cwd":"/tmp","role":"implementer","resumeSessionId":"sess-1"}`,
		`{"cwd":"/tmp","resumeSessionId":"sess-2"}`,
		`{"cwd":"/tmp","capability":"frontier","resumeSessionId":"sess-3"}`,
	} {
		m := spawnVia(t, url, "tok-operator", params, got)
		if m["resumeSessionId"] == nil {
			t.Errorf("params %s reached the provider without its resume: %v", params, m)
		}
	}
}

// A fresh ROLE that starts a NEW session is ordinary. The arm refuses inheriting
// a conversation; it does not restrict reviewers.
func TestAFreshRoleStartingANewSessionIsAdmitted(t *testing.T) {
	url, got, _ := ceilingServer(t, freshResolver)
	m := spawnVia(t, url, "tok-operator", `{"cwd":"/tmp","role":"reviewer","capability":"deep_reviewer"}`, got)
	if m["role"] != "reviewer" || m["capability"] != "deep_reviewer" {
		t.Errorf("a reviewer starting a new session was damaged: %v", m)
	}
}

// THE FEDERATED HOP. The claim the whole placement rests on is that
// methodSanitizers is the single dispatch table for call() AND federatedCall(),
// so a rule written once covers a spawn crossing to a peer. A reviewer that
// could inherit a session by naming `hub:work/agents.spawn` would be one hop
// away from walking around this entirely.
func TestTheFreshRefusalAlsoCoversTheFederatedHop(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetSpawnCeiling(freshResolver, nil)

	fed := &fakeFed{got: make(chan json.RawMessage, 4)}
	srv.SetFederation(fed)

	caller := dialClientToken(t, url, "host-secret")
	caller.send(Frame{Op: "call", ID: "f1", Method: "hub:work/agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp","role":"reviewer","resumeSessionId":"peer-sess-1"}`)})
	f := caller.readUntil("error")
	if !strings.Contains(f.Error, "peer-sess-1") {
		t.Errorf("a FEDERATED spawn crossed the hop with its resume intact; error was %q", f.Error)
	}
	select {
	case raw := <-fed.got:
		t.Fatalf("the refused spawn was forwarded to the peer anyway: %s", raw)
	default:
	}
}

// The HOST token is exempt from the caller-tier clamp — it is the control plane.
// It is NOT exempt from this: freshness is a property of the work, not of the
// credential, and the desktop dispatching a review through the bus has the same
// reason to want an independent reviewer as anyone else.
func TestTheHostTokenIsNotExemptFromTheFreshRefusal(t *testing.T) {
	url, got, _ := ceilingServer(t, freshResolver)
	caller := dialClientToken(t, url, "host-secret")
	caller.send(Frame{Op: "call", ID: "s1", Method: "agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp","role":"reviewer","resumeSessionId":"sess-host"}`)})
	caller.readUntil("error")
	select {
	case raw := <-got:
		t.Fatalf("the host token walked around the freshness refusal: %s", raw)
	default:
	}
}

// With NO routing layer wired at all, nothing here fires: the arm is a reading
// of routing.yaml and a hub built without one has no matrix to read.
func TestWithoutARoutingLayerNoResumeIsRefused(t *testing.T) {
	url, got, _ := ceilingServer(t, nil)
	m := spawnVia(t, url, "tok-operator", `{"cwd":"/tmp","role":"reviewer","resumeSessionId":"sess-1"}`, got)
	if m["resumeSessionId"] != "sess-1" {
		t.Errorf("something refused a resume with no ceiling resolver wired: %v", m)
	}
}
