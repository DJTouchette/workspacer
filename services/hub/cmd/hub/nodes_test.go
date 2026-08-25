package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/nodes"
)

// The credential this whole file exists to keep out of reach.
const flyTokenLiteral = "FlyV1_fm2_THE_TOKEN_THAT_SPENDS_MONEY"

func nodesRegistry(t *testing.T) (string, *nodes.Supervisor) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "nodes.json")
	body := `[{"id":"den","label":"Fly node (den)","fly":{"app":"wks-node-den","machineId":"17811944b12345","token":"` +
		flyTokenLiteral + `","baseUrl":"https://api.machines.dev"}}]`
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	entries, err := nodes.LoadFile(path)
	if err != nil {
		t.Fatalf("LoadFile: %v", err)
	}
	sup := nodes.New(nodes.Options{Nodes: entries, Bus: deadBus{}, Logf: func(string, ...any) {}})
	return path, sup
}

// deadBus answers "nothing is registered" — the shape a stopped node has.
type deadBus struct{}

func (deadBus) ProbeBrain(context.Context) (nodes.Probe, error) {
	return nodes.Probe{}, nodes.ErrNoProvider
}
func (deadBus) BrainProviderRegistered() bool { return false }
func (deadBus) EvictBrainProvider() bool      { return false }

// THE TOKEN CANNOT BE REACHED THROUGH ANY CLIENT-FACING METHOD.
//
// Both bus methods are invoked exactly as the router would invoke them, their
// answers are rendered exactly as the wire would render them, and the
// credential is searched for in the bytes. Rendering rather than
// field-checking is the point: it catches a leak through a field nobody has
// added yet.
func TestNoNodesMethodEverRendersTheFlyToken(t *testing.T) {
	_, sup := nodesRegistry(t)
	ctx := context.Background()

	listed, err := nodesList(sup)(nil)
	if err != nil {
		t.Fatalf("nodes.list: %v", err)
	}
	assertNoToken(t, "nodes.list", listed)

	// A trusted caller reaching nodes.wake. There is no cloud client
	// registered for this node, so this is the refusal path — which is
	// exactly where an implementation is most tempted to quote the config it
	// could not use.
	woken, err := nodesWake(ctx, sup)(bus.CallerIdentity{Trusted: true}, json.RawMessage(`{"id":"den"}`))
	if err == nil {
		t.Fatal("expected a refusal: no cloud client is configured in this test")
	}
	if strings.Contains(err.Error(), flyTokenLiteral) {
		t.Errorf("the nodes.wake ERROR carried the Fly token: %s", err)
	}
	assertNoToken(t, "nodes.wake", woken)

	// And the sleep half, on the same refusal path. A stop has more values in
	// it than a start, which is exactly why an implementation is tempted to
	// echo the coordinates back when it cannot use them.
	slept, err := nodesSleep(ctx, sup)(bus.CallerIdentity{Trusted: true}, json.RawMessage(`{"id":"den"}`))
	if err == nil {
		t.Fatal("expected a refusal: no cloud client is configured in this test")
	}
	if strings.Contains(err.Error(), flyTokenLiteral) {
		t.Errorf("the nodes.sleep ERROR carried the Fly token: %s", err)
	}
	assertNoToken(t, "nodes.sleep", slept)
}

func assertNoToken(t *testing.T, method string, v any) {
	t.Helper()
	raw, err := json.Marshal(v)
	if err != nil {
		t.Fatalf("%s: marshal: %v", method, err)
	}
	for _, leak := range []string{flyTokenLiteral, "wks-node-den", "17811944b12345", "api.machines.dev"} {
		if strings.Contains(string(raw), leak) {
			t.Errorf("%s disclosed %q: %s", method, leak, raw)
		}
	}
}

// The hub READS the registry and never writes it. There is no round-trip that
// could rewrite a hand-edited comment, reorder entries, or — the one that
// matters — persist a resolved token back into a file that only held a path.
func TestTheHubNeverWritesTheNodeRegistry(t *testing.T) {
	path, sup := nodesRegistry(t)
	before, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	statBefore, _ := os.Stat(path)

	sup.Reconcile(context.Background())
	_, _ = nodesList(sup)(nil)
	_, _ = nodesWake(context.Background(), sup)(bus.CallerIdentity{Trusted: true}, json.RawMessage(`{"id":"den"}`))
	_, _ = nodesSleep(context.Background(), sup)(bus.CallerIdentity{Trusted: true}, json.RawMessage(`{"id":"den"}`))

	after, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(before) != string(after) {
		t.Fatalf("nodes.json was rewritten:\nbefore: %s\nafter:  %s", before, after)
	}
	statAfter, _ := os.Stat(path)
	if !statBefore.ModTime().Equal(statAfter.ModTime()) {
		t.Error("nodes.json's mtime changed — something wrote it")
	}
}

// ---- the tier decision, pinned ------------------------------------------

// nodes.wake is HOST AUTHORITY ONLY. Waking starts a billable machine and this
// hub has no way to stop one, so a phone tier gets the state and not the
// button. Two independent refusals, and this is the one at the handler.
func TestNodesWakeRefusesEveryUntrustedCaller(t *testing.T) {
	_, sup := nodesRegistry(t)
	h := nodesWake(context.Background(), sup)
	for _, caller := range []bus.CallerIdentity{
		{},                              // a view- or triage-tier connection
		{TokenID: "plugin-fingerprint"}, // a plugin token
	} {
		if _, err := h(caller, json.RawMessage(`{"id":"den"}`)); err == nil {
			t.Fatalf("nodes.wake answered an untrusted caller %+v", caller)
		} else if !strings.Contains(err.Error(), "host authority") {
			t.Errorf("refusal did not say why: %v", err)
		}
	}
}

// And this is the other one: the tier allowlists. Unlisted is denied, so the
// pin is that nodes.wake appears in NEITHER scoped tier and nodes.list appears
// in view.
func TestTheScopedTiersSeeNodeStateAndCannotSpendMoney(t *testing.T) {
	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage} {
		methods := scope.Methods()
		if !containsString(methods, "nodes.list") {
			t.Errorf("%s tier cannot read nodes.list — a phone would render a sleeping node as a dead one", scope)
		}
		if containsString(methods, "nodes.wake") {
			t.Errorf("%s tier holds nodes.wake — that tier can now start billable machines", scope)
		}
		// The stop verb is refused for a reason of its OWN, not by symmetry:
		// a stop lands on a machine somebody may be typing at and ends the work
		// in flight on it. "It only turns things off" is destructive, not
		// smaller — a phone tier gets neither button.
		if containsString(methods, "nodes.sleep") {
			t.Errorf("%s tier holds nodes.sleep — that tier can now shut down a machine somebody is working on", scope)
		}
	}
	if !containsString(authtoken.ScopeOperator.Methods(), "*") {
		t.Skip("operator tier is not the wildcard any more; re-derive this pin")
	}
}

func containsString(ss []string, want string) bool {
	for _, s := range ss {
		if s == want {
			return true
		}
	}
	return false
}

// ---- registration shape --------------------------------------------------

// Both methods must be registered in cmd/hub/main.go with LITERAL names. The
// brain's headless-completeness guard parses RegisterLocal names out of that
// one file and capspec's composition bearings grep for the
// nodesTrusted("nodes.wake", …) call shape — a registration anywhere else, or
// behind a variable, is invisible to both and this was got wrong once already.
func TestNodeMethodsAreRegisteredWithLiteralNamesInMainGo(t *testing.T) {
	main, err := os.ReadFile(filepath.Join("main.go"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		`srv.RegisterLocal("nodes.list"`,
		`srv.RegisterLocalIdent("nodes.wake"`,
		`srv.RegisterLocalIdent("nodes.sleep"`,
	} {
		if !strings.Contains(string(main), want) {
			t.Errorf("cmd/hub/main.go does not contain %s — the completeness and composition guards both parse THIS file", want)
		}
	}
	gate, err := os.ReadFile(filepath.Join("nodes.go"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{`nodesTrusted("nodes.wake"`, `nodesTrusted("nodes.sleep"`} {
		if !strings.Contains(string(gate), want) {
			t.Errorf("cmd/hub/nodes.go does not contain %s — capspec's composition bearing reads THIS file for that call shape", want)
		}
	}
}

// nodes.wake without an id is a refusal, not a wake of something arbitrary.
func TestNodesWakeNeedsAnId(t *testing.T) {
	_, sup := nodesRegistry(t)
	h := nodesWake(context.Background(), sup)
	for _, params := range []json.RawMessage{nil, json.RawMessage(`{}`), json.RawMessage(`{"id":"   "}`)} {
		if _, err := h(bus.CallerIdentity{Trusted: true}, params); err == nil {
			t.Errorf("nodes.wake accepted params %q with no node id", params)
		}
	}
}

// ---- nodes.sleep: the capability this branch adds ------------------------

// THE SAME GATE, AND FOR A REASON OF ITS OWN.
//
// A stop is not the cheap direction of a wake. It lands on a machine somebody
// may be typing at and it ends the work in flight on it, so it is refused to
// exactly the callers nodes.wake is refused to — and it would be even if it
// cost nothing at all.
func TestNodesSleepRefusesEveryUntrustedCaller(t *testing.T) {
	_, sup := nodesRegistry(t)
	h := nodesSleep(context.Background(), sup)
	for _, caller := range []bus.CallerIdentity{
		{},                              // a view- or triage-tier connection
		{TokenID: "plugin-fingerprint"}, // a plugin token, however its manifest reads
	} {
		if _, err := h(caller, json.RawMessage(`{"id":"den"}`)); err == nil {
			t.Fatalf("nodes.sleep answered an untrusted caller %+v — that caller can now shut down a machine somebody is working on", caller)
		} else if !strings.Contains(err.Error(), "host authority") {
			t.Errorf("refusal did not say why: %v", err)
		}
	}
}

func TestNodesSleepNeedsAnId(t *testing.T) {
	_, sup := nodesRegistry(t)
	h := nodesSleep(context.Background(), sup)
	for _, params := range []json.RawMessage{nil, json.RawMessage(`{}`), json.RawMessage(`{"id":"   "}`)} {
		if _, err := h(bus.CallerIdentity{Trusted: true}, params); err == nil {
			t.Errorf("nodes.sleep accepted params %q with no node id", params)
		}
	}
}

// THE MUTATION GUARD ON THE PARAMETER SURFACE.
//
// A stop has knobs a start does not — a signal, a drain window, and (if anyone
// were careless) coordinates. Every one of them must come from the hub's own
// side: nodes.json for the machine, the supervisor's tunables for the signal
// and the window. The caller supplies an `id` that SELECTS a row, and nothing
// else it sends may reach the cloud API.
//
// This test feeds the handler every field a careless implementation might have
// been tempted to honour, and asserts the answer is the same as the bare call's
// — including the refusal, which is the one for THIS registry's node and not
// for the app/machine the caller named. Break it by adding a `Signal` or an
// `App` field to the request struct and honouring it, and this fails.
func TestNodesSleepIgnoresEveryCallerSuppliedFieldButTheID(t *testing.T) {
	_, sup := nodesRegistry(t)
	h := nodesSleep(context.Background(), sup)

	_, bare := h(bus.CallerIdentity{Trusted: true}, json.RawMessage(`{"id":"den"}`))
	if bare == nil {
		t.Fatal("expected a refusal: no cloud client is configured in this test")
	}
	loaded := json.RawMessage(`{
	  "id":"den",
	  "signal":"SIGKILL",
	  "timeout":"0s",
	  "force":true,
	  "app":"someone-elses-app",
	  "machineId":"deadbeefcafe",
	  "baseUrl":"http://attacker.example",
	  "token":"FlyV1_fm2_A_TOKEN_THE_CALLER_BROUGHT",
	  "nodes":[{"id":"den","fly":{"app":"x","machineId":"y"}}]
	}`)
	got, err := h(bus.CallerIdentity{Trusted: true}, loaded)
	if err == nil {
		t.Fatal("the loaded call was ACCEPTED where the bare one was refused — the handler acted on something the caller sent")
	}
	if err.Error() != bare.Error() {
		t.Errorf("a caller-supplied field changed the outcome.\n bare: %v\nloaded: %v", bare, err)
	}
	for _, leak := range []string{"someone-elses-app", "deadbeefcafe", "attacker.example", "A_TOKEN_THE_CALLER_BROUGHT", "SIGKILL"} {
		if strings.Contains(err.Error(), leak) {
			t.Errorf("the refusal echoed the caller's %q back: %v", leak, err)
		}
	}
	assertNoToken(t, "nodes.sleep", got)

	// An unknown id is still an unknown id however much else is in the object.
	if _, err := h(bus.CallerIdentity{Trusted: true}, json.RawMessage(`{"id":"not-in-the-registry","app":"wks-node-den","machineId":"17811944b12345"}`)); err == nil {
		t.Error("nodes.sleep accepted an id that is not in the registry, alongside coordinates that ARE — the id selects a row and the coordinates must be inert")
	} else if !strings.Contains(err.Error(), "unknown node") {
		t.Errorf("refusal for an unregistered id = %v, want an unknown-node refusal", err)
	}
}

// The two verbs must not drift into different gates. Both call nodesTrusted
// with their OWN literal name — the shape capspec's bearing greps for — and a
// copy-paste that left the wrong literal in the second one would satisfy a
// "does it refuse" test while pointing capspec's evidence at the wrong method.
func TestBothActingNodeVerbsNameThemselvesAtTheirGate(t *testing.T) {
	src, err := os.ReadFile(filepath.Join("nodes.go"))
	if err != nil {
		t.Fatal(err)
	}
	body := string(src)
	for _, method := range []string{"nodes.wake", "nodes.sleep"} {
		call := `nodesTrusted("` + method + `"`
		if strings.Count(body, call) != 1 {
			t.Errorf("cmd/hub/nodes.go contains %d occurrences of %s, want exactly 1 — capspec's bearing for %s rests on that call site being there and being unique", strings.Count(body, call), call, method)
		}
	}
	// And the gate itself must still refuse. A guard that stopped checking
	// would leave every call site above intact and green.
	if err := nodesTrusted("nodes.sleep", bus.CallerIdentity{}); err == nil {
		t.Fatal("nodesTrusted admits an untrusted caller")
	}
	if err := nodesTrusted("nodes.sleep", bus.CallerIdentity{Trusted: true}); err != nil {
		t.Fatalf("nodesTrusted refuses the host: %v", err)
	}
}
