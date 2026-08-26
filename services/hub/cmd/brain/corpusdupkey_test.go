package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/extinput"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// A DUPLICATE KEY IS THE ONE CORPUS DEFECT EVERY LOADER AGREES TO IGNORE.
//
// encoding/json keeps the LAST occurrence. So does JSON.parse. So does
// serde_json's default Map. Which means a case object that carries the same
// field twice loads cleanly in all three languages with one of the two values
// silently gone, and every guard in this directory — which validates the
// DECODED document — is looking at a map where the collision already happened.
//
// It is not hypothetical. `path-containment-cases.json` carried two `note` keys
// on the git.* entry until 2026-08-25: one sentence about which field is the
// guarded one and one about git.diff's second boundary. Only the second was
// ever readable, in any language, and nothing anywhere went red.
//
// The check has to run over the TOKEN STREAM, before a decoder folds the
// duplicate away, which is why it is a separate sweep from
// TestEveryFixtureBlockIsDeclaredAndClosed rather than another rule inside it.
func TestNoContractFixtureCarriesADuplicateKey(t *testing.T) {
	dir, err := sweepguard.RepoPath("contracts")
	if err != nil {
		t.Fatalf("locate contracts/: %v", err)
	}
	// extinput.ReadDir for contractsFixtures' reason: the LISTING is this
	// sweep's input, and an out-of-module os.ReadDir is not in cmd/go's test
	// cache key — a new fixture would arrive to `ok (cached)`.
	entries, err := extinput.ReadDir(dir)
	if err != nil {
		t.Fatalf("read contracts/: %v", err)
	}
	var tally sweepguard.Tally
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		t.Run(e.Name(), func(t *testing.T) {
			raw, err := sweepguard.ReadRepoFile("contracts", e.Name())
			if err != nil {
				t.Fatalf("read %s: %v", e.Name(), err)
			}
			tally.Ran("other")
			for _, dup := range duplicateJSONKeys(t, raw) {
				t.Errorf("contracts/%s carries the key %s twice — every parser this corpus is read by keeps only the LAST one, so one of the two values is already invisible to Go, TypeScript and Rust alike",
					e.Name(), dup)
			}
		})
	}
	// Its own floor, and a HIGHER one than the vocabulary sweep's next door:
	// that sweep exempts the four fixtures carrying no case blocks, and a
	// duplicate key is a defect in a schema or a lock file too.
	if err := tally.RequireEvery("the contracts duplicate-key sweep", contractsJSONFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

// contractsJSONFloor is how many .json files contracts/ holds, exemptions
// included. A floor, not an equality: adding a fixture does not need a bump,
// but a listing that came back short is a sweep that quietly stopped covering
// files it claims to cover.
const contractsJSONFloor = 16

// duplicateJSONKeys walks the token stream and returns every object key that
// appears more than once in the same object, addressed by path.
func duplicateJSONKeys(t *testing.T, raw []byte) []string {
	t.Helper()
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var dups []string
	var walk func(path string)
	walk = func(path string) {
		tok, err := dec.Token()
		if err != nil {
			if err != io.EOF {
				t.Fatalf("tokenise %s: %v", path, err)
			}
			return
		}
		delim, ok := tok.(json.Delim)
		if !ok {
			return // a scalar; nothing below it
		}
		switch delim {
		case '{':
			seen := map[string]bool{}
			for dec.More() {
				k, err := dec.Token()
				if err != nil {
					t.Fatalf("tokenise a key under %s: %v", path, err)
				}
				name := fmt.Sprint(k)
				if seen[name] {
					dups = append(dups, path+"."+name)
				}
				seen[name] = true
				walk(path + "." + name)
			}
			if _, err := dec.Token(); err != nil { // the closing brace
				t.Fatalf("close %s: %v", path, err)
			}
		case '[':
			for i := 0; dec.More(); i++ {
				walk(fmt.Sprintf("%s[%d]", path, i))
			}
			if _, err := dec.Token(); err != nil { // the closing bracket
				t.Fatalf("close %s: %v", path, err)
			}
		}
	}
	walk("$")
	sort.Strings(dups)
	return dups
}

// TestTheDuplicateKeyGuardIsFalsifiable is the answer this directory asks of
// every guard: break the thing on purpose and require the complaint. Without
// it, a walk that silently stopped descending would report a clean corpus.
func TestTheDuplicateKeyGuardIsFalsifiable(t *testing.T) {
	for _, tc := range []struct {
		name string
		body string
		want string
	}{
		{"a top-level key twice", `{"a":1,"a":2}`, "$.a"},
		{"a key inside a nested object", `{"outer":{"note":"x","note":"y"}}`, "$.outer.note"},
		{"a key inside an object in an array", `{"cases":[{"why":"a"},{"why":"b","why":"c"}]}`, "$.cases[1].why"},
		{"the same key in sibling objects is NOT a duplicate", `{"a":{"k":1},"b":{"k":2}}`, ""},
		{"a repeated VALUE is not a repeated key", `{"a":"x","b":"x"}`, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := duplicateJSONKeys(t, []byte(tc.body))
			if tc.want == "" {
				if len(got) != 0 {
					t.Fatalf("reported %v for a document with no duplicate key", got)
				}
				return
			}
			if len(got) != 1 || got[0] != tc.want {
				t.Fatalf("reported %v, want exactly [%s] — the walk is not descending where it claims to", got, tc.want)
			}
		})
	}
}
