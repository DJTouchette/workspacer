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
	} {
		if !strings.Contains(string(main), want) {
			t.Errorf("cmd/hub/main.go does not contain %s — the completeness and composition guards both parse THIS file", want)
		}
	}
	gate, err := os.ReadFile(filepath.Join("nodes.go"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(gate), `nodesTrusted("nodes.wake"`) {
		t.Error(`cmd/hub/nodes.go does not contain nodesTrusted("nodes.wake", …) — capspec's bearing for nodes.wake reads this file`)
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
