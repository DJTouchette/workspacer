package authtoken

import (
	"encoding/json"
	"path/filepath"
	"slices"
	"testing"
)

// THE PROVIDER TIER, held to what it claims to be.
//
// `provider` is not a rung on the view ⊂ triage ⊂ operator ladder — it is an
// orthogonal axis: one method it may CALL, and one authority no human tier has
// (registering as the answerer of a call). Each half is pinned here, because
// the failure mode of a tier is silent: nothing errors when a tier quietly
// acquires a method, it just starts working.

func TestParseScopeAcceptsProvider(t *testing.T) {
	for _, in := range []string{"provider", "PROVIDER", "  provider\t"} {
		got, err := ParseScope(in)
		if err != nil {
			t.Fatalf("ParseScope(%q) = %v", in, err)
		}
		if got != ScopeProvider {
			t.Fatalf("ParseScope(%q) = %q, want provider", in, got)
		}
	}
	if _, err := ParseScope("providers"); err == nil {
		t.Fatal("ParseScope(\"providers\") must fail closed — an unknown tier grants nothing")
	}
}

// The call surface. `provider` holds layout.get and NOTHING else: it is the one
// method a headless node actually calls (cmd/brain/main.go, the
// fleet-visibility read). Everything else it does is answering, not asking.
func TestProviderCallSurfaceIsExactlyLayoutGet(t *testing.T) {
	got := ScopeProvider.Methods()
	if !slices.Equal(got, []string{"layout.get"}) {
		t.Fatalf("ScopeProvider.Methods() = %v, want exactly [layout.get]. Adding a method here needs a call site in cmd/brain to justify it — a provider that needs to CALL something is asking for the operator ladder, which is a different question from being allowed to ANSWER.", got)
	}
}

// The relationship to the human ladder, stated as the invariant rather than as
// prose: provider's call surface is a strict SUBSET of the smallest human tier,
// and it shares nothing with what triage adds. A provider token is not "a
// small operator" — on the acting side it is less than view.
func TestProviderIsStrictlyBelowViewAndDisjointFromTriage(t *testing.T) {
	view := ScopeView.Methods()
	for _, m := range ScopeProvider.Methods() {
		if !slices.Contains(view, m) {
			t.Errorf("provider holds %q and view does not. The provider tier is defined as a strict subset of view on the CALL plane; a method here that view lacks means the tier grew an acting surface nobody priced.", m)
		}
	}
	if len(ScopeProvider.Methods()) >= len(view) {
		t.Errorf("provider now holds %d methods and view holds %d — the subset is no longer strict", len(ScopeProvider.Methods()), len(view))
	}
	for _, m := range triageMethods {
		if slices.Contains(ScopeProvider.Methods(), m) {
			t.Errorf("provider holds %q, which is triage's acting surface (approve, message, interrupt, push, upload). A headless node answers calls; it does not act on a human's behalf.", m)
		}
	}
	// And the star that means host identity is not here. ScopedIdent.operator()
	// scans Methods for "*" and promotes such a token to `trusted`.
	for _, m := range ScopeProvider.Methods() {
		if m == "*" {
			t.Fatal("ScopeProvider.Methods() contains \"*\" — the bus reads a star in Methods as HOST IDENTITY (ScopedIdent.operator), so every node token would be promoted to trusted and the tier would grant strictly more than the operator token it replaces")
		}
	}
}

// The mint default is load-bearing, not a shrug. A grant narrower than what the
// provider registers puts the brain in a permanent 5-second re-register loop:
// the `registered` ack carries the accepted list and no reason, so the retry
// cannot tell "another live connection owns this" (retry is right) from "your
// token may not have this" (retry is forever). Until the ack carries a reason,
// the only safe mint is the whole register surface.
func TestProviderMintDefaultsToTheWholeRegisterSurface(t *testing.T) {
	file := filepath.Join(t.TempDir(), "tokens.json")
	rec, err := Mint(file, ScopeProvider, "fly-node")
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(rec.Provides, []string{"*"}) {
		t.Fatalf("Mint(provider).Provides = %v, want [\"*\"]. Narrowing the default is a permanent 5s re-register loop on every deployed node, and it presents as a working node rather than as a refusal — see Record.Provides.", rec.Provides)
	}

	// It must survive the file, not just the return value: the hub reads the
	// STORE, and a field that round-trips in memory and vanishes on disk is a
	// node that works until the hub restarts.
	recs, err := Load(file)
	if err != nil {
		t.Fatal(err)
	}
	if len(recs) != 1 || !slices.Equal(recs[0].Provides, []string{"*"}) {
		t.Fatalf("after Save/Load: %+v", recs)
	}
	got, ok := NewStore(file).Lookup(rec.Token)
	if !ok || !slices.Equal(got.ProvidesGrant(), []string{"*"}) {
		t.Fatalf("Store.Lookup(provider).ProvidesGrant() = %v (ok=%v)", got.ProvidesGrant(), ok)
	}
}

// No other tier acquires a register grant by minting.
func TestMintGivesNoRegisterGrantToTheHumanTiers(t *testing.T) {
	for _, sc := range []Scope{ScopeView, ScopeTriage, ScopeOperator} {
		file := filepath.Join(t.TempDir(), "tokens.json")
		rec, err := Mint(file, sc, "x")
		if err != nil {
			t.Fatal(err)
		}
		if len(rec.Provides) != 0 {
			t.Errorf("Mint(%s).Provides = %v, want empty — only the provider tier registers capabilities", sc, rec.Provides)
		}
	}
}

// THE TIER IS THE GATE, NOT THE FIELD. tokens.json is a plain file that the
// CLI, the desktop and the hub all rewrite, so `provides` can appear on a
// record that has no business with it — a hand edit, a bad migration, two
// stores merged. Read raw, that would make a read-only phone token the answerer
// of claude.approve: whoever holds the provider slot receives every subsequent
// caller's params and returns whatever it likes.
func TestProvidesGrantIsRefusedToEveryTierButProvider(t *testing.T) {
	for _, sc := range []Scope{ScopeView, ScopeTriage, ScopeOperator, Scope("bogus"), Scope("")} {
		rec := Record{Token: "t", Scope: sc, Provides: []string{"*", "claude.approve"}}
		if got := rec.ProvidesGrant(); got != nil {
			t.Errorf("Record{Scope:%q, Provides:[* claude.approve]}.ProvidesGrant() = %v, want nil. The tier decides who may register, so a provides field on any other record must grant nothing.", sc, got)
		}
	}
	ok := Record{Token: "t", Scope: ScopeProvider, Provides: []string{"sessions.snapshot"}}
	if got := ok.ProvidesGrant(); !slices.Equal(got, []string{"sessions.snapshot"}) {
		t.Fatalf("provider ProvidesGrant() = %v, want [sessions.snapshot]", got)
	}
	// The returned slice must be a copy: the store hands the same Record out to
	// every connection, and a caller that appends to this grant would widen it
	// for everyone.
	g := ok.ProvidesGrant()
	g[0] = "*"
	if ok.Provides[0] != "sessions.snapshot" {
		t.Fatal("ProvidesGrant() aliased the record's own slice — a caller can mutate one connection's grant into every connection's")
	}
}

// Wire shape, the same omitempty contract profilesAllowed/yoloAllowed carry:
// present as an array on a provider record, ABSENT everywhere else — never null
// or []. TWIN: RemoteTokenRecord.provides (apps/desktop ipcTypes.ts), whose
// normalizeRecord must preserve it or the next desktop mint deletes the node's
// credential.
func TestProvidesWireShape(t *testing.T) {
	b, err := json.Marshal(Record{Token: "t", Scope: ScopeView})
	if err != nil {
		t.Fatal(err)
	}
	if m := map[string]any{}; json.Unmarshal(b, &m) == nil {
		if _, present := m["provides"]; present {
			t.Errorf("a view record serialized `provides`: %s", b)
		}
	}
	b, err = json.Marshal(Record{Token: "t", Scope: ScopeProvider, Provides: []string{"*"}})
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatal(err)
	}
	if got, ok := m["provides"].([]any); !ok || len(got) != 1 || got[0] != "*" {
		t.Fatalf("provider record wire shape = %s, want provides:[\"*\"]", b)
	}
}

// The CLI's Load→Save rewrites (token create / token revoke) must not strip the
// grant off a record they are not touching — the same rule Role, Plugins and
// ProfilesAllowed each earned a field for.
func TestRevokingAnotherTokenPreservesTheProviderGrant(t *testing.T) {
	file := filepath.Join(t.TempDir(), "tokens.json")
	node, err := Mint(file, ScopeProvider, "fly-node")
	if err != nil {
		t.Fatal(err)
	}
	phone, err := Mint(file, ScopeTriage, "phone")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := Revoke(file, phone.Token); err != nil {
		t.Fatal(err)
	}
	recs, err := Load(file)
	if err != nil {
		t.Fatal(err)
	}
	if len(recs) != 1 || recs[0].Token != node.Token {
		t.Fatalf("after revoking the phone: %+v", recs)
	}
	if !slices.Equal(recs[0].Provides, []string{"*"}) {
		t.Fatalf("the node's register grant did not survive a rewrite of tokens.json: %+v — the node 401s into a register loop on its next reconnect", recs[0])
	}
}
