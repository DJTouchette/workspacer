package main

// contracts/provider-parity-cases.json, Go side.
//
// Path containment has its own corpus. This one pins the other half of "the same
// bus call must give the same answer whichever provider ran": the ORDER a list
// comes back in, and how each side reads a scalar that is not the type it
// expected. Both were live divergences — see the fixture's header.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// parityFixtureRel names the fixture from the REPO ROOT, not from this package
// dir. It is read through mustReadRepoFile so cmd/go's test cache sees it; a
// "../../../.." path handed to os.ReadFile is dropped from the cache key.
const parityFixtureRel = "contracts/provider-parity-cases.json"

type parityFixture struct {
	Order []struct {
		Name     string   `json:"name"`
		Input    []string `json:"input"`
		Expected []string `json:"expected"`
		Why      string   `json:"why"`
	} `json:"order"`
	Scalar []struct {
		Name     string `json:"name"`
		Value    any    `json:"value"`
		Expected string `json:"expected"`
	} `json:"scalar"`
	Suffix []struct {
		Name     string `json:"name"`
		Value    string `json:"value"`
		Suffix   string `json:"suffix"`
		Fold     bool   `json:"fold"`
		Expected string `json:"expected"`
		Why      string `json:"why"`
	} `json:"suffix"`
}

func loadParityFixture(t *testing.T) parityFixture {
	t.Helper()
	raw := mustReadRepoFile(t, "contracts", "provider-parity-cases.json")
	var fx parityFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", parityFixtureRel, err)
	}
	if len(fx.Order) == 0 || len(fx.Scalar) == 0 || len(fx.Suffix) == 0 {
		t.Fatal("a silently empty parity fixture pins nothing")
	}
	return fx
}

// The three blocks' sizes today. A `len(...) == 0` check is the only floor this
// loader had, and it is met by a block that lost every case but one.
const (
	parityOrderFloor  = 4
	parityScalarFloor = 7
	paritySuffixFloor = 10
)

func TestProviderParityFixtureCases(t *testing.T) {
	fx := loadParityFixture(t)

	var order, scalar, suffix sweepguard.Tally
	for _, c := range fx.Order {
		t.Run("order/"+c.Name, func(t *testing.T) {
			order.Ran("other")
			got := append([]string(nil), c.Input...)
			sort.SliceStable(got, func(i, j int) bool { return got[i] < got[j] })
			if strings.Join(got, "\x00") != strings.Join(c.Expected, "\x00") {
				t.Fatalf("byte order drifted\n  got:  %v\n  want: %v\n  why:  %s", got, c.Expected, c.Why)
			}
		})
	}
	for _, c := range fx.Scalar {
		t.Run("scalar/"+c.Name, func(t *testing.T) {
			scalar.Ran("other")
			if got := str(c.Value); got != c.Expected {
				t.Fatalf("str(%#v) = %q, want %q", c.Value, got, c.Expected)
			}
		})
	}
	for _, c := range fx.Suffix {
		t.Run("suffix/"+c.Name, func(t *testing.T) {
			got := strings.TrimSuffix(c.Value, c.Suffix)
			if c.Fold {
				if c.Suffix != ".md" {
					suffix.Skip("a fold case whose suffix is not .md; the twin shipped here is trimMDSuffix")
					t.Skipf("the folding twin shipped here is trimMDSuffix, which is .md-specific")
				}
				got = trimMDSuffix(c.Value)
			}
			suffix.Ran("other")
			if got != c.Expected {
				t.Fatalf("trim(%q, %q, fold=%v) = %q, want %q\n  why: %s", c.Value, c.Suffix, c.Fold, got, c.Expected, c.Why)
			}
		})
	}
	// One floor per block: these are three independent corpora sharing a file,
	// and a total would let a block empty itself behind the other two.
	for _, f := range []struct {
		what  string
		tally *sweepguard.Tally
		floor int
	}{
		{"the provider-parity order block", &order, parityOrderFloor},
		{"the provider-parity scalar block", &scalar, parityScalarFloor},
		{"the provider-parity suffix block", &suffix, paritySuffixFloor},
	} {
		// The suffix block has one skip gate (a non-.md fold case), so it is
		// held to the enumerated floor; the other two have none.
		var err error
		if f.tally == &suffix {
			err = f.tally.RequireCorpus(f.what, f.floor, 0, 0)
		} else {
			err = f.tally.RequireEvery(f.what, f.floor)
		}
		if err != nil {
			t.Error(err)
		}
		t.Logf("%s: %s", f.what, f.tally.String())
	}
}

// The helpers above pinned in ISOLATION are not enough — the divergence shipped
// at the CALL SITES. These drive the real listers with the fixture's own
// `order` input, so a lister that sorts some other way is caught even though the
// comparison function itself is right.
func TestListersUseTheFixtureOrdering(t *testing.T) {
	fx := loadParityFixture(t)
	c := fx.Order[0]

	t.Run("library.list titles", func(t *testing.T) {
		sandbox, err := filepath.EvalSymlinks(t.TempDir())
		if err != nil {
			t.Fatal(err)
		}
		t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
		t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
		dir := libraryGlobalDir()
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		// This asserts on the WHOLE list, so the starters must not join it.
		suppressLibrarySeed(t)
		for i, title := range c.Input {
			body := "---\ntitle: " + title + "\n---\n\nx\n"
			if err := os.WriteFile(filepath.Join(dir, string(rune('a'+i))+".md"), []byte(body), 0o644); err != nil {
				t.Fatal(err)
			}
		}
		var got []string
		for _, it := range listLibrary("", allowAnyLibraryFile, libraryFilter{}) {
			got = append(got, it.Title)
		}
		if strings.Join(got, "\x00") != strings.Join(c.Expected, "\x00") {
			t.Fatalf("library.list ordering drifted\n  got:  %v\n  want: %v", got, c.Expected)
		}
	})

	t.Run("fs.listDir directory names", func(t *testing.T) {
		root := t.TempDir()
		for _, name := range c.Input {
			if err := os.MkdirAll(filepath.Join(root, name), 0o755); err != nil {
				t.Fatal(err)
			}
		}
		res, err := listHostDir(root)
		if err != nil {
			t.Fatal(err)
		}
		if strings.Join(res.Dirs, "\x00") != strings.Join(c.Expected, "\x00") {
			t.Fatalf("fs.listDir ordering drifted\n  got:  %v\n  want: %v", res.Dirs, c.Expected)
		}
	})
}

// The store listers must survive a scalar that is not a string. The desktop twin
// called .localeCompare on it, which THREW inside the comparator, and the
// function-level catch returned an empty list — every well-formed row vanished
// with the odd one. Both sides now coerce and sort byte-wise, so the odd row
// sorts last and everything else still lists.
func TestStoreListersSurviveANonStringScalar(t *testing.T) {
	sandbox := tempConfigHome(t)
	t.Setenv("APPDATA", sandbox)
	if err := os.MkdirAll(layoutsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(sessionsDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	for i, ts := range []string{"'2026-03-01T00:00:00.000Z'", "'2026-02-01T00:00:00.000Z'", "'2026-01-01T00:00:00.000Z'"} {
		name := "real" + string(rune('1'+i))
		layout := "id: " + name + "\nname: " + name + "\ncreatedAt: " + ts + "\nagents: []\n"
		if err := os.WriteFile(filepath.Join(layoutsDir(), name+".yaml"), []byte(layout), 0o644); err != nil {
			t.Fatal(err)
		}
		session := "name: " + name + "\ntimestamp: " + ts + "\n"
		if err := os.WriteFile(filepath.Join(sessionsDir(), name+".yaml"), []byte(session), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	// The poison, in both shapes: a bare number, and an UNQUOTED ISO date — which
	// js-yaml 4 parses to a Date, so it needs no attacker at all, just a hand
	// edit. "aaa" so it lands first in readdir order, which is what made the
	// desktop's insertion sort put it in the `b` position.
	poisonLayout := "id: aaa\nname: Aaa\ncreatedAt: 5\nagents: []\n"
	if err := os.WriteFile(filepath.Join(layoutsDir(), "aaa.yaml"), []byte(poisonLayout), 0o644); err != nil {
		t.Fatal(err)
	}
	poisonSession := "name: aaa\ntimestamp: 2026-03-01T00:00:00.000Z\n"
	if err := os.WriteFile(filepath.Join(sessionsDir(), "aaa.yaml"), []byte(poisonSession), 0o644); err != nil {
		t.Fatal(err)
	}

	var layoutIDs []string
	for _, l := range listLayouts() {
		layoutIDs = append(layoutIDs, str(l["id"]))
	}
	if want := []string{"real1", "real2", "real3", "aaa"}; strings.Join(layoutIDs, ",") != strings.Join(want, ",") {
		t.Errorf("layouts.list = %v, want %v — the odd row sorts last, it does not take the list with it", layoutIDs, want)
	}

	var sessionNames []string
	for _, s := range listSavedSessions() {
		sessionNames = append(sessionNames, s.Name)
	}
	if want := []string{"real1", "real2", "real3", "aaa"}; strings.Join(sessionNames, ",") != strings.Join(want, ",") {
		t.Errorf("sessions.list = %v, want %v", sessionNames, want)
	}
}

// The two library-item field derivations, at their CALL SITE. Both were live
// divergences from the desktop, and both are invisible to the helper-level cases
// above because readLibraryDir does not use trimMDSuffix or firstNonEmpty by
// name in a way any fixture case reaches.
func TestLibraryItemFieldsMatchTheDesktop(t *testing.T) {
	sandbox, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(sandbox, "config"))
	t.Setenv("APPDATA", filepath.Join(sandbox, "config"))
	dir := libraryGlobalDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	// The entry FILTER is case-insensitive (`strings.ToLower(name)` HasSuffix
	// ".md"), so every one of these is listed — and trimming only ".md"/".MD"
	// left ".Md" and ".mD" on, minting "readme-md" here where the desktop's
	// /\.md$/i mints "readme". Two ids for one file, and on the desktop side the
	// collision with a plain readme.md dropped an item out of the list entirely.
	for name, title := range map[string]string{
		"notes.md":  "Notes",
		"readme.Md": "Readme mixed",
		"guide.mD":  "Guide",
		"spec.MD":   "Spec",
		// A whitespace-only title: `firstNonEmpty` used to test `v != ""` and
		// served three spaces, while the desktop's
		// `typeof t === 'string' && t.trim() ? t : id` served the id — a blank
		// row in the library picker under the default catalog delegation and a
		// named row without it.
		"wsp.md": "   ",
	} {
		body := "---\ntitle: \"" + title + "\"\n---\n\nx\n"
		if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	got := map[string]string{}
	for _, it := range listLibrary("", allowAnyLibraryFile, libraryFilter{}) {
		got[it.ID] = it.Title
	}
	want := map[string]string{
		"notes":  "Notes",
		"readme": "Readme mixed",
		"guide":  "Guide",
		"spec":   "Spec",
		"wsp":    "wsp", // the id, not three spaces
	}
	for id, title := range want {
		if g, ok := got[id]; !ok {
			t.Errorf("no item with id %q; got ids %v", id, got)
		} else if g != title {
			t.Errorf("item %q title = %q, want %q", id, g, title)
		}
	}
}
