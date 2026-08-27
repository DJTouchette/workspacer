package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// writeLibraryFixture drops one markdown item into a library dir.
func writeLibraryFixture(t *testing.T, dir, name, body string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
}

// The Go half of contracts/dispatch-template-params-cases.json. The TS half is
// apps/desktop/src/main/lib/dispatchTemplate.test.ts; both must produce the
// SAME param list for the same template body, because both answer library.list
// and a manager must not learn a different set of placeholders depending on
// which provider ran.

const dispatchParamsFixtureRel = "contracts/dispatch-template-params-cases.json"

// dispatchParamsOwnerKey is this implementation's key in the fixture's `owners`
// map. Dropping the file out of `owners` would otherwise run every case and
// prove nothing about whether THIS copy is still on the hook.
const dispatchParamsOwnerKey = "services/hub/cmd/brain/dispatchparams.go"

type dispatchParamsCase struct {
	Name     string          `json:"name"`
	Template string          `json:"template"`
	Expect   []dispatchParam `json:"expect"`
	Why      string          `json:"why"`
}

type dispatchParamsFixture struct {
	Owners map[string]string    `json:"owners"`
	Cases  []dispatchParamsCase `json:"cases"`
}

// dispatchParamsCorpusFloor is the size of the corpus today. A floor of "not
// zero" is met by a corpus that lost every case but one.
const dispatchParamsCorpusFloor = 18

func TestDispatchTemplateParamsContractCases(t *testing.T) {
	raw := mustReadRepoFile(t, "contracts", "dispatch-template-params-cases.json")
	var fx dispatchParamsFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", dispatchParamsFixtureRel, err)
	}
	if fx.Owners[dispatchParamsOwnerKey] == "" {
		t.Fatalf("owners must name %q; got %v", dispatchParamsOwnerKey, fx.Owners)
	}
	if len(fx.Cases) == 0 {
		t.Fatalf("%s decoded to zero cases — a silently empty corpus guards nothing", dispatchParamsFixtureRel)
	}

	var tally sweepguard.Tally
	for _, c := range fx.Cases {
		t.Run(c.Name, func(t *testing.T) {
			tally.Ran("other")
			got := dispatchTemplateParams(c.Template)
			if len(got) != len(c.Expect) {
				t.Fatalf("dispatchTemplateParams(%q) returned %d params %+v, want %d %+v\n  why: %s",
					c.Template, len(got), got, len(c.Expect), c.Expect, c.Why)
			}
			for i, want := range c.Expect {
				// ORDER is part of the contract, not an accident of iteration:
				// the list is what a caller reads top-down to fill a template.
				if got[i] != want {
					t.Errorf("dispatchTemplateParams(%q)[%d] = %+v, want %+v\n  why: %s",
						c.Template, i, got[i], want, c.Why)
				}
			}
		})
	}
	if err := tally.RequireEvery("the dispatch-template params corpus", dispatchParamsCorpusFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

// The advertised shape and the ENFORCED shape are the same parse — on this side
// there is no renderer to check against (agents.spawn's `template` stays
// declined here, see spawnParamsDeclined), so what this pins instead is that a
// listed dispatch item's params really are derived from its stored body.
func TestDispatchItemsCarryTheirParsedParams(t *testing.T) {
	tempConfigHome(t)
	suppressLibrarySeed(t)
	gdir := libraryGlobalDir()
	writeLibraryFixture(t, gdir, "ship.md",
		"---\ntitle: Ship\nkind: dispatch\n---\n\nSHIP: {{task}}\nDeliver: {{delivery:open a PR}} in {{cwd}}\n")
	// A non-dispatch item with the very same body carries NO params: the field
	// is a property of the kind, not of the text.
	writeLibraryFixture(t, gdir, "prompt.md",
		"---\ntitle: Prompt\nkind: prompt\n---\n\nSHIP: {{task}}\n")

	items := listLibrary("", allowAnyLibraryFile, libraryFilter{})
	byID := map[string]libraryItem{}
	for _, it := range items {
		byID[it.ID] = it
	}
	ship, ok := byID["ship"]
	if !ok {
		t.Fatalf("no ship item; got %v", byID)
	}
	want := []dispatchParam{
		{Name: "task", Required: true},
		{Name: "delivery", Required: false, Default: "open a PR"},
	}
	if len(ship.Params) != len(want) {
		t.Fatalf("ship.Params = %+v, want %+v", ship.Params, want)
	}
	for i := range want {
		if ship.Params[i] != want[i] {
			t.Errorf("ship.Params[%d] = %+v, want %+v", i, ship.Params[i], want[i])
		}
	}
	if p := byID["prompt"].Params; p != nil {
		t.Errorf("a kind 'prompt' item carries params %+v; the field is dispatch-only", p)
	}
	// Never written back. serializeItem emits libFrontmatter, which models no
	// `params` key, so the derived field cannot become file state — and a
	// round trip through saveLibrary is where it would if it could.
	saved, err := (&registry{}).saveLibrary(context.Background(), libraryInput{
		Scope: "global", ID: "ship", Title: "Ship", Kind: "dispatch",
		Body: "SHIP: {{task}}\nDeliver: {{delivery:open a PR}}",
	})
	if err != nil {
		t.Fatalf("saveLibrary: %v", err)
	}
	if len(saved.Params) != 2 || saved.Params[0].Name != "task" {
		t.Errorf("save echoed params %+v, want the same list the next list() reports", saved.Params)
	}
	raw, err := os.ReadFile(filepath.Join(gdir, "ship.md"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "params:") {
		t.Errorf("the stored template gained a params key:\n%s", raw)
	}
}

func TestLibraryFilterNarrowsTheListing(t *testing.T) {
	tempConfigHome(t)
	suppressLibrarySeed(t)
	gdir := libraryGlobalDir()
	writeLibraryFixture(t, gdir, "ship.md", "---\ntitle: Ship\nkind: dispatch\n---\n\n{{task}}\n")
	writeLibraryFixture(t, gdir, "scout.md", "---\ntitle: Scout\nkind: dispatch\n---\n\n{{question}}\n")
	writeLibraryFixture(t, gdir, "notes.md", "---\ntitle: Notes\nkind: prompt\n---\n\nplain\n")

	ids := func(f libraryFilter) []string {
		var out []string
		for _, it := range listLibrary("", allowAnyLibraryFile, f) {
			out = append(out, it.ID)
		}
		return out
	}
	// Sorted by title, as always: Notes, Scout, Ship.
	if got := ids(libraryFilter{}); len(got) != 3 {
		t.Fatalf("unfiltered listing = %v, want 3 items", got)
	}
	if got := ids(libraryFilter{Kind: "dispatch"}); len(got) != 2 || got[0] != "scout" || got[1] != "ship" {
		t.Errorf("kind filter = %v, want [scout ship] — a filtered listing is the unfiltered one minus rows, order intact", got)
	}
	if got := ids(libraryFilter{ID: "ship"}); len(got) != 1 || got[0] != "ship" {
		t.Errorf("id filter = %v, want [ship]", got)
	}
	// ANDed, not ORed.
	if got := ids(libraryFilter{Kind: "prompt", ID: "ship"}); len(got) != 0 {
		t.Errorf("kind+id filter = %v, want none — the two fields are ANDed", got)
	}
	if got := ids(libraryFilter{ID: "nope"}); len(got) != 0 {
		t.Errorf("id filter for a missing item = %v, want none", got)
	}
}

// The handler half of the filters: the vocabulary check and the empty-string
// reading, which are the two places the two providers could disagree about the
// same call.
func TestLibraryListHandlerFilters(t *testing.T) {
	cwd, _ := libraryCwdWithConfigDir(t)
	suppressLibrarySeed(t)
	gdir := libraryGlobalDir()
	writeLibraryFixture(t, gdir, "ship.md", "---\ntitle: Ship\nkind: dispatch\n---\n\n{{task}}\n")
	writeLibraryFixture(t, gdir, "notes.md", "---\ntitle: Notes\nkind: prompt\n---\n\nplain\n")

	list := func(t *testing.T, params string) string {
		t.Helper()
		reg := registryWithCwd(t, cwd)
		res, err := reg.handle(context.Background(), "library.list", json.RawMessage(params))
		if err != nil {
			t.Fatalf("library.list%s: %v", params, err)
		}
		return string(res)
	}

	if got := list(t, `{}`); !strings.Contains(got, `"ship"`) || !strings.Contains(got, `"notes"`) {
		t.Fatalf("unfiltered listing lost a row: %s", got)
	}
	// The params ride the listing, so a manager can read them without the body.
	if got := list(t, `{"kind":"dispatch"}`); !strings.Contains(got, `"params"`) ||
		!strings.Contains(got, `"name":"task"`) || strings.Contains(got, `"notes"`) {
		t.Fatalf("kind filter: %s", got)
	}
	if got := list(t, `{"id":"notes"}`); strings.Contains(got, `"ship"`) {
		t.Fatalf("id filter kept a row it should have dropped: %s", got)
	}
	// EMPTY STRINGS ARE "NO FILTER". Go's omitempty makes an omitted facade
	// field arrive as "", and the desktop twin reads it the same way — treating
	// it as a kind named "" would make the same call answer differently
	// depending on which provider ran.
	if got := list(t, `{"kind":"","id":""}`); !strings.Contains(got, `"ship"`) ||
		!strings.Contains(got, `"notes"`) {
		t.Fatalf("empty-string filters must mean no filter: %s", got)
	}
	// An unknown kind is refused out loud, never answered with an empty list.
	reg := registryWithCwd(t, cwd)
	if _, err := reg.handle(context.Background(), "library.list",
		json.RawMessage(`{"kind":"dispatchh"}`)); err == nil {
		t.Fatal("an unknown kind must be refused, not answered with an empty list")
	}
}
