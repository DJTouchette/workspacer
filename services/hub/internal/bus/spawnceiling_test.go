package bus

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// THE CLAMP, AT THE REAL DISPATCH POINT. Every test here runs the whole path —
// websocket handshake, tier check, sanitizeSpawnParams, forward to a registered
// provider — and reads what the PROVIDER received, because the invariant is
// about the params on the far side of the hub and not about any predicate in
// isolation. Same discipline as profilegrant_test.go next door.

// ceilingServer wires a bus with a host token, three scoped tiers, a plugin
// token, an injected ceiling resolver and an audit sink.
//
// The resolver is a STAND-IN for internal/routing, deliberately: the bus must
// hold no matrix, so what is proven here is that the router applies whatever it
// is told, and internal/routing's own tests prove what it is told is right.
func ceilingServer(t *testing.T, ceiling SpawnCeilingFunc) (url string, got chan json.RawMessage, audits *[]SpawnRecord) {
	t.Helper()
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetScopedTokenLookup(func(tok string) (ScopedIdent, bool) {
		switch tok {
		case "tok-operator":
			return ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		case "tok-triage-spawner":
			// A HAND-EDITED record: triage with agents.spawn added. The tier
			// lists do not grant it today, which is exactly why the clamp is a
			// belt — an authority ladder enforced only by a method list is
			// enforced only until somebody adds a method.
			return ScopedIdent{Scope: "triage", Methods: append(authtoken.ScopeTriage.Methods(), "agents.spawn")}, true
		}
		return ScopedIdent{}, false
	})
	srv.RegisterPluginToken("plug-tok", "test.plugin",
		[]capspec.Grant{{Method: "agents.spawn"}}, capspec.EventGrants{})

	recs := &[]SpawnRecord{}
	srv.SetSpawnCeiling(ceiling, func(r SpawnRecord) { *recs = append(*recs, r) })

	got = make(chan json.RawMessage, 8)
	provider := dialClientToken(t, url, "host-secret")
	provider.send(Frame{Op: "register", Methods: []string{"agents.spawn"}})
	provider.readUntil("registered")
	go func() {
		for {
			f, ok := provider.tryRead("call")
			if !ok {
				return
			}
			got <- f.Params
			provider.send(Frame{Op: "result", ID: f.ID, Result: json.RawMessage(`{"ok":true}`)})
		}
	}()
	return url, got, recs
}

func scrubbedList(t *testing.T, m map[string]any) []string {
	t.Helper()
	raw, ok := m["escalationScrubbed"].([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(raw))
	for _, v := range raw {
		s, _ := v.(string)
		out = append(out, s)
	}
	return out
}

// namesField is the "was this reported to the caller" check. Named rather than
// spelled `has` because the test bodies shadow that word with the two-value map
// idiom on every other line.
func namesField(list []string, want string) bool {
	for _, s := range list {
		if s == want {
			return true
		}
	}
	return false
}

// THE HEADLINE: a spawn above the directory's capability ceiling reaches the
// provider clamped, WITHOUT its model, and says so in the answer channel.
//
// The model half is what stops the clamp being a relabelling: a spawn that kept
// `model: fable` after `capability` was lowered to `frontier` has had a label
// changed, not a limit applied.
func TestCapabilityAboveTheCeilingIsClampedAndTheModelGoesWithIt(t *testing.T) {
	url, got, audits := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		if req.Capability == "frontier_plus" {
			return SpawnCeilingVerdict{
				Key: "default", MaxCapability: "frontier", MaxToolScope: "operator",
				CapabilityRefused: true, Capability: "frontier",
				Because: []string{"the stand-in ceiling caps this directory at frontier"},
			}
		}
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "frontier", MaxToolScope: "operator"}
	})

	m := spawnVia(t, url, "tok-operator",
		`{"cwd":"/tmp","capability":"frontier_plus","model":"fable","effort":"high","role":"judge","decisionId":"rd_abc"}`, got)

	if m["capability"] != "frontier" {
		t.Errorf("capability reached the provider as %v, want frontier", m["capability"])
	}
	if _, has := m["model"]; has {
		t.Errorf("the model the refused capability chose survived the clamp: %v", m)
	}
	if _, has := m["effort"]; has {
		t.Errorf("the effort survived the clamp: %v", m)
	}
	// NO SILENT DOWNGRADES: all three named in the answer channel.
	scrubbed := scrubbedList(t, m)
	for _, want := range []string{"capability", "model", "effort"} {
		if !namesField(scrubbed, want) {
			t.Errorf("escalationScrubbed %v does not name %q — the downgrade is visible only in a log the caller cannot read", scrubbed, want)
		}
	}
	// Unrelated params are untouched, and the recorded metadata rides through.
	if m["cwd"] != "/tmp" || m["role"] != "judge" || m["decisionId"] != "rd_abc" {
		t.Errorf("the clamp damaged params it has no business with: %v", m)
	}

	if len(*audits) != 1 {
		t.Fatalf("the spawn produced %d audit records, want 1", len(*audits))
	}
	rec := (*audits)[0]
	if rec.DecisionID != "rd_abc" || rec.Role != "judge" {
		t.Errorf("the audit record cannot be joined to the decision: %+v", rec)
	}
	if !rec.Ceiling.CapabilityRefused || !namesField(rec.Scrubbed, "model") {
		t.Errorf("the audit record does not record the refusal: %+v", rec)
	}
}

// A spawn UNDER the ceiling is untouched, and is still recorded: "we looked, and
// the ceiling allowed it" is a row worth having.
func TestSpawnUnderTheCeilingIsUntouchedAndStillRecorded(t *testing.T) {
	url, got, audits := ceilingServer(t, func(SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "frontier", MaxToolScope: "operator"}
	})
	m := spawnVia(t, url, "tok-operator",
		`{"cwd":"/tmp","capability":"balanced","model":"gpt-5.6-terra","effort":"high","toolScope":"operator"}`, got)

	if m["capability"] != "balanced" || m["model"] != "gpt-5.6-terra" || m["effort"] != "high" {
		t.Errorf("an allowed spawn was modified: %v", m)
	}
	if m["toolScope"] != "operator" {
		t.Errorf("an operator caller's operator toolScope was clamped: %v", m)
	}
	if _, present := m["escalationScrubbed"]; present {
		t.Errorf("nothing was taken but the answer claims it was: %v", m)
	}
	if len(*audits) != 1 || (*audits)[0].Ceiling.Refused() {
		t.Errorf("audit records = %+v", *audits)
	}
}

// INVARIANT 1a: a caller may not grant a child a tier above its own. It needs no
// routing file at all, which is why the resolver here refuses nothing.
func TestCallerCannotGrantAToolTierAboveItsOwn(t *testing.T) {
	permissive := func(SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "frontier_plus", MaxToolScope: "operator"}
	}
	url, got, _ := ceilingServer(t, permissive)

	m := spawnVia(t, url, "tok-triage-spawner", `{"cwd":"/tmp","toolScope":"operator"}`, got)
	if m["toolScope"] != "triage" {
		t.Errorf("a triage credential handed its child %v — a caller cannot grant more authority than it holds", m["toolScope"])
	}
	if !namesField(scrubbedList(t, m), "toolScope") {
		t.Errorf("the tier downgrade was not reported to the caller: %v", m)
	}
	// At or below its own tier is untouched.
	m = spawnVia(t, url, "tok-triage-spawner", `{"cwd":"/tmp","toolScope":"view"}`, got)
	if m["toolScope"] != "view" {
		t.Errorf("a triage credential asking for view was clamped to %v", m["toolScope"])
	}
	// The HOST token is the control plane and is deliberately exempt: a process
	// holding it could rewrite tokens.json, so clamping it here is theater.
	m = spawnVia(t, url, "host-secret", `{"cwd":"/tmp","toolScope":"operator"}`, got)
	if m["toolScope"] != "operator" {
		t.Errorf("the host token was clamped to %v", m["toolScope"])
	}
}

// The legacy `mcpFacade: true` spelling MEANS operator, and a clamp that only
// rewrote `toolScope` would be walked around by one boolean.
func TestTheLegacyFacadeFlagIsClampedToo(t *testing.T) {
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		v := SpawnCeilingVerdict{Key: "/tmp", MaxCapability: "frontier_plus", MaxToolScope: "view"}
		if req.ToolScope == "operator" {
			v.ToolScopeRefused, v.ToolScope = true, "view"
			v.Because = []string{"the stand-in ceiling caps this directory at view"}
		}
		return v
	})

	m := spawnVia(t, url, "tok-operator", `{"cwd":"/tmp","mcpFacade":true}`, got)
	if v, present := m["mcpFacade"]; present && v == true {
		t.Errorf("mcpFacade:true survived a view ceiling — the legacy spelling walks around the clamp: %v", m)
	}
	if m["toolScope"] != "view" {
		t.Errorf("the clamped tier is %v, want view: %v", m["toolScope"], m)
	}
	if !namesField(scrubbedList(t, m), "mcpFacade") {
		t.Errorf("the legacy flag was removed without saying so: %v", m)
	}

	// BOTH SPELLINGS AT ONCE. `toolScope` wins over `mcpFacade` on both
	// providers (cmd/brain/facade.go facadeScope, claudeSpawn.ts facadeScope), so
	// the clamped toolScope is the operative one and the stale legacy flag is
	// left alone rather than being deleted on a guess. This case exists because
	// the opposite resolution order would make the clamp a no-op, and nothing
	// else in the repo pins that order from this side.
	m = spawnVia(t, url, "tok-operator", `{"cwd":"/tmp","toolScope":"operator","mcpFacade":true}`, got)
	if m["toolScope"] != "view" {
		t.Errorf("with both spellings present the clamped tier is %v, want view: %v", m["toolScope"], m)
	}
}

// THE CWD REACHES THE RESOLVER CANONICALIZED. CeilingFor on the other side is a
// LEXICAL ancestor match, so a symlinked spelling that arrived unresolved would
// walk straight around a per-directory ceiling. The router does the walk; this
// watches it happen through the real dispatch path.
func TestTheCeilingResolverIsHandedACanonicalCwd(t *testing.T) {
	dir := t.TempDir()
	real := filepath.Join(dir, "locked")
	if err := os.MkdirAll(real, 0o755); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(dir, "shortcut")
	if err := os.Symlink(real, link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	// The canonical form of the temp dir itself (macOS puts /var behind /private).
	canonicalReal, ok := CanonicalizeRoot(real)
	if !ok {
		t.Fatalf("could not canonicalize the fixture directory %s", real)
	}

	seen := make(chan SpawnCeilingRequest, 4)
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		seen <- req
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "frontier_plus", MaxToolScope: "operator"}
	})

	spawnVia(t, url, "tok-operator", `{"cwd":`+jsonString(link)+`}`, got)
	req := <-seen
	if !req.CwdResolved {
		t.Fatalf("the router reported the cwd unresolved: %+v", req)
	}
	if req.CanonicalCwd != canonicalReal {
		t.Errorf("the resolver was handed %q, want the symlink-resolved %q — a lexical ceiling looked up on the caller's spelling is a ceiling a symlink walks around",
			req.CanonicalCwd, canonicalReal)
	}
}

// An UNRESOLVABLE cwd must still be asked about, so the DEFAULT ceiling applies.
// Treating "we could not resolve it" as "unconstrained" would make the ceiling
// optional for anyone willing to spell the path badly.
func TestAnUnresolvableCwdStillAsksForACeiling(t *testing.T) {
	seen := make(chan SpawnCeilingRequest, 4)
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		seen <- req
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "cheap", MaxToolScope: "view",
			CapabilityRefused: true, Capability: "cheap",
			Because: []string{"the default ceiling applies to a directory with no entry of its own"}}
	})

	for _, params := range []string{
		`{"capability":"frontier"}`,                      // no cwd at all
		`{"cwd":"relative/dir","capability":"frontier"}`, // not absolute
		`{"cwd":"","capability":"frontier"}`,             // empty
	} {
		m := spawnVia(t, url, "tok-operator", params, got)
		req := <-seen
		if req.CwdResolved || req.CanonicalCwd != "" {
			t.Errorf("%s: reported resolved=%v cwd=%q", params, req.CwdResolved, req.CanonicalCwd)
		}
		if m["capability"] != "cheap" {
			t.Errorf("%s: an unresolvable cwd escaped the default ceiling: %v", params, m)
		}
	}
}

// THE FEDERATED HOP, for free. methodSanitizers is the single dispatch table for
// call() AND federatedCall(), which is the whole reason this clamp lives here;
// the sibling drift test proves the table is shared, and this proves the CLAMP
// rides it — a forwarded spawn re-enters this router on the link's connection
// and must be judged like any other.
func TestTheCeilingClampAlsoCoversTheFederatedHop(t *testing.T) {
	url, srv := rpcServerWith(t)
	srv.SetToken("host-secret")
	srv.SetSpawnCeiling(func(SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "balanced", MaxToolScope: "operator",
			CapabilityRefused: true, Capability: "balanced",
			Because: []string{"stand-in"}}
	}, nil)

	fed := &fakeFed{got: make(chan json.RawMessage, 4)}
	srv.SetFederation(fed)

	caller := dialClientToken(t, url, "host-secret")
	caller.send(Frame{Op: "call", ID: "f1", Method: "hub:work/agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp","capability":"frontier_plus","model":"fable"}`)})
	caller.readUntil("result")

	var m map[string]any
	if err := json.Unmarshal(<-fed.got, &m); err != nil {
		t.Fatalf("forwarded params did not decode: %v", err)
	}
	if m["capability"] != "balanced" {
		t.Errorf("a FEDERATED spawn crossed the hop at %v — the clamp did not ride the shared sanitizer table", m["capability"])
	}
	if _, present := m["model"]; present {
		t.Errorf("the model survived the federated clamp: %v", m)
	}
	if !namesField(func() []string {
		raw, _ := m["escalationScrubbed"].([]any)
		out := make([]string, 0, len(raw))
		for _, v := range raw {
			s, _ := v.(string)
			out = append(out, s)
		}
		return out
	}(), "model") {
		t.Errorf("the federated downgrade was not reported: %v", m)
	}
}

// With NO ceiling layer wired at all (a hub built without routing, and every
// test in this package that does not set one), nothing capability-shaped is
// clamped and the caller-tier half still applies — the two are independent by
// construction, and conflating them would make Invariant 1a depend on a file.
func TestWithoutARoutingLayerOnlyTheCallerTierClampApplies(t *testing.T) {
	url, got, _ := ceilingServer(t, nil)

	m := spawnVia(t, url, "tok-operator", `{"cwd":"/tmp","capability":"frontier_plus","model":"fable"}`, got)
	if m["capability"] != "frontier_plus" || m["model"] != "fable" {
		t.Errorf("something was clamped with no ceiling resolver wired: %v", m)
	}
	m = spawnVia(t, url, "tok-triage-spawner", `{"cwd":"/tmp","toolScope":"operator"}`, got)
	if m["toolScope"] != "triage" {
		t.Errorf("the caller-tier clamp needs no routing file and did not fire: %v", m)
	}
}

// jsonString quotes a path for embedding in a params literal.
func jsonString(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}

// Guard against the sentence in the header drifting: every clamp reason must
// carry the remedy, because the file is the only way to change a ceiling.
func TestClampReasonsNameTheFileAsTheRemedy(t *testing.T) {
	url, got, audits := ceilingServer(t, func(SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{Key: "default", MaxCapability: "cheap", MaxToolScope: "operator",
			CapabilityRefused: true, Capability: "cheap",
			Because: []string{"routing.yaml's ceilings.default caps this directory at cheap"}}
	})
	spawnVia(t, url, "tok-operator", `{"cwd":"/tmp","capability":"frontier"}`, got)
	if len(*audits) != 1 {
		t.Fatalf("audits = %d", len(*audits))
	}
	if !strings.Contains(strings.Join((*audits)[0].Ceiling.Because, " "), "routing.yaml") {
		t.Errorf("the recorded reason does not name where the ceiling came from: %+v", (*audits)[0].Ceiling)
	}
}

// A DENIED VERDICT STOPS THE SPAWN, and it is recorded on the way out.
//
// The routing layer sets Denied when the CEILING ITSELF cannot be read — a row
// naming an unrankable capability or a tier the security model does not have.
// Clamping is not available in that case (there is nothing to clamp TO), and
// admitting would make a typo in the policy file the quietest way to delete the
// policy. So the router refuses, and the audit still gets its row: a refusal
// nobody logged is the one event an operator cannot explain.
func TestADeniedCeilingRefusesTheSpawnAndStillRecordsIt(t *testing.T) {
	url, got, audits := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{
			Key: "default", MaxCapability: "frontierr", Denied: true,
			Because: []string{"ceilings.default.max_capability is \"frontierr\", which capability_ranks: does not rank"},
		}
	})

	caller := dialClientToken(t, url, "tok-operator")
	caller.send(Frame{Op: "call", ID: "s1", Method: "agents.spawn",
		Params: json.RawMessage(`{"cwd":"/tmp","capability":"balanced"}`)})
	f := caller.readUntil("error")
	if f.ID != "s1" {
		t.Fatalf("error frame id %q, want s1", f.ID)
	}
	if !strings.Contains(f.Error, "frontierr") {
		t.Errorf("the caller must be told which value to fix; got %q", f.Error)
	}
	select {
	case raw := <-got:
		t.Fatalf("a DENIED spawn reached the provider anyway: %s", raw)
	default:
	}
	if len(*audits) != 1 {
		t.Fatalf("a denied spawn wrote %d audit rows, want 1", len(*audits))
	}
	if !(*audits)[0].Ceiling.Denied {
		t.Error("the audit row does not record that this spawn was denied")
	}
}

// THE ROUTER WRITES THE SAFE TUPLE. The routing layer names what the permitted
// capability resolves to; this is the half that puts it on the wire, so the
// provider is left no omitted field to resolve from its own configured default.
func TestAClampedCapabilityArrivesWithAnExplicitSafeModel(t *testing.T) {
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		if req.Capability != "frontier_plus" {
			return SpawnCeilingVerdict{Key: "default", MaxCapability: "frontier"}
		}
		return SpawnCeilingVerdict{
			Key: "default", MaxCapability: "frontier",
			CapabilityRefused: true, Capability: "frontier",
			Provider: "claude", Model: "opus", Effort: "high",
			Because: []string{"the stand-in ceiling caps this directory at frontier"},
		}
	})

	m := spawnVia(t, url, "tok-operator",
		`{"cwd":"/tmp","capability":"frontier_plus","model":"fable","effort":"max"}`, got)

	if m["capability"] != "frontier" {
		t.Errorf("capability reached the provider as %v, want frontier", m["capability"])
	}
	if m["model"] != "opus" {
		t.Errorf("model reached the provider as %v, want the routed replacement opus — an ABSENT model is the provider's own default, which is the hole this closes", m["model"])
	}
	if m["effort"] != "high" {
		t.Errorf("effort reached the provider as %v, want high", m["effort"])
	}
	if m["provider"] != "claude" {
		t.Errorf("provider reached the provider as %v, want claude", m["provider"])
	}
	// The caller still learns what was taken: replaced is not the same as kept.
	scrub := scrubbedList(t, m)
	for _, f := range []string{"capability", "model", "effort"} {
		if !namesField(scrub, f) {
			t.Errorf("%q was replaced without being reported in escalationScrubbed: %v", f, scrub)
		}
	}
}

// A caller's EXPLICIT provider is never overwritten. The routing layer only ever
// answers with a tuple on the provider the spawn was already for, and the router
// must not paper over the case where it answered with a different one.
func TestTheClampNeverOverwritesAnExplicitProvider(t *testing.T) {
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{
			Key: "default", MaxCapability: "frontier",
			CapabilityRefused: true, Capability: "frontier",
			Provider: "codex", Model: "gpt-5.6-sol", Effort: "high",
		}
	})
	m := spawnVia(t, url, "tok-operator",
		`{"cwd":"/tmp","provider":"claude","capability":"frontier_plus"}`, got)
	if m["provider"] != "claude" {
		t.Errorf("the clamp swapped an explicitly named harness: provider reached the provider as %v", m["provider"])
	}
}

// And when the routing layer names NO replacement, the old behaviour stands:
// drop the model rather than invent one.
func TestWithNoRoutedReplacementTheModelIsStillDropped(t *testing.T) {
	url, got, _ := ceilingServer(t, func(req SpawnCeilingRequest) SpawnCeilingVerdict {
		return SpawnCeilingVerdict{
			Key: "default", MaxCapability: "balanced",
			CapabilityRefused: true, Capability: "balanced",
		}
	})
	m := spawnVia(t, url, "tok-operator",
		`{"cwd":"/tmp","provider":"copilot","capability":"frontier_plus","model":"something","effort":"high"}`, got)
	if _, has := m["model"]; has {
		t.Errorf("a model was invented for a provider the matrix does not serve: %v", m)
	}
	if _, has := m["effort"]; has {
		t.Errorf("effort survived with no replacement tuple: %v", m)
	}
}
