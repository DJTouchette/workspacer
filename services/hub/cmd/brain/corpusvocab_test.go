package main

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/extinput"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// ---------------------------------------------------------------------------
// THE CORPUS VOCABULARY GUARD — every fixture, every block.
//
// path-containment-cases.json grew a `vocabulary` block that validated tokens,
// groups and deniedBy. It protected less than it looked like it did:
//
//   - It covered ONE array — `cases` — of ONE fixture. spawnCwds, methods,
//     checkUse, paramShapes, projectDirNames and asciiFold got nothing, and the
//     other six fixtures in contracts/ got nothing at all.
//   - So sessionFilenames.cases still carried a bare `expect: "refuse"` with no
//     reason — the EXACT species the deniedBy work existed to kill, sitting one
//     block away in the same file. `refuse` alone is satisfied by a refusal for
//     any reason at all, including a resolver that refuses everything, and the
//     case named "traverses a symlink out of the sessions dir" is in fact
//     refused by the basename rule and never reaches a symlink. Nothing said so.
//
// This guard is driven by each fixture's own `vocabulary.blocks` registry rather
// than by a list in here, and closes in BOTH directions: every array-of-objects
// block in the file must be declared, and every declaration must name a real
// block. A new block is a failure until someone writes down what its cases owe.
//
// TWIN: apps/desktop/src/main/services/contractsVocabulary.test.ts runs the same
// checks over the same files. TestBothCorpusVocabularyLoadersExist below is what
// stops that from being a claim in a comment.
// ---------------------------------------------------------------------------

// vocabCheckIDs are the names of the checks this validator performs. Both
// loaders declare the list, and each asserts the other's source still carries
// every ID — the cross-loader existence guard the three per-loader vocabulary
// tests never had.
var vocabCheckIDs = []string{
	"blocks-declared",
	"blocks-exist",
	"required-fields",
	"verdict-vocabulary",
	"verdict-reason-required",
	"verdict-reason-declared",
	"verdict-reason-forbidden",
	"reason-vocabulary-used",
	"unique-case-names",
	"token-references",
	"unknown-fields",
	"optional-used",
	"block-loaders",
}

type vocabBlockSpec struct {
	Why      string   `json:"why"`
	Required []string `json:"required"`
	// Optional closes the field set from the other side. A field NAME the loaders
	// act on was closed by nothing: renaming `configDirVia` by one character left
	// every suite green, because encoding/json and JSON.parse both ignore an
	// unknown key — and that field is uniquely SILENT, since dropping the symlink
	// indirection does not flip either case's verdict, so both cases kept passing
	// while exercising nothing they claimed to.
	Optional []string `json:"optional"`
	// Nested does the same for the sub-keys of a field whose shape is a SCHEMA
	// (path-containment's `tree`) rather than a data payload (deepmerge's
	// `target`). Opt-in per block for exactly that reason.
	Nested map[string][]string `json:"nested"`
	// Loaders are the tests that read THIS BLOCK, "<repo-relative file>::<needle>".
	// The per-fixture loader count is a per-FILE guard and a block could lose
	// every loader it had while the file kept eight; see contracts_test.go
	// TestEveryDeclaredBlockLoaderStillExists, which is what checks these resolve.
	Loaders     []string                   `json:"loaders"`
	VerdictFld  string                     `json:"verdictField"`
	Verdicts    map[string]vocabVerdictDef `json:"verdicts"`
	unusedGuard struct{}
}

type vocabVerdictDef struct {
	Requires []string `json:"requires"`
	Forbids  []string `json:"forbids"`
	Reasons  string   `json:"reasons"`
}

// validateFixtureVocabulary is the whole check, as a pure function over the
// decoded document. It is a function and not a test body on purpose: a test body
// cannot be fed a deliberately broken fixture, and a check that has never been
// SEEN to fail is indistinguishable from one that cannot. See
// TestTheVocabularyGuardIsFalsifiable.
func validateFixtureVocabulary(name string, doc map[string]any) []string {
	var problems []string
	add := func(format string, args ...any) {
		problems = append(problems, name+": "+fmt.Sprintf(format, args...))
	}

	vocab, _ := doc["vocabulary"].(map[string]any)
	if vocab == nil {
		add("no `vocabulary` block — every fixture must declare one, or nothing holds its blocks to anything [blocks-declared]")
		return problems
	}
	rawBlocks, _ := vocab["blocks"].(map[string]any)
	if len(rawBlocks) == 0 {
		add("`vocabulary.blocks` is empty — the registry is what makes this guard non-vacuous [blocks-declared]")
		return problems
	}
	specs := map[string]vocabBlockSpec{}
	for key, raw := range rawBlocks {
		if key == "_comment" {
			continue
		}
		encoded, err := json.Marshal(raw)
		if err != nil {
			add("vocabulary.blocks.%s is not an object", key)
			continue
		}
		var spec vocabBlockSpec
		if err := json.Unmarshal(encoded, &spec); err != nil {
			add("vocabulary.blocks.%s does not decode: %v", key, err)
			continue
		}
		specs[key] = spec
	}

	// Every array-of-objects block in the document, by dotted path.
	found := map[string][]map[string]any{}
	var walk func(v any, path string)
	walk = func(v any, path string) {
		switch t := v.(type) {
		case map[string]any:
			for k, e := range t {
				next := k
				if path != "" {
					next = path + "." + k
				}
				if path == "vocabulary" || strings.HasPrefix(path, "vocabulary.") {
					continue // the registry describes blocks; it is not one
				}
				walk(e, next)
			}
		case []any:
			if len(t) == 0 {
				return
			}
			rows := make([]map[string]any, 0, len(t))
			for _, e := range t {
				row, ok := e.(map[string]any)
				if !ok {
					return // a list of strings/prose, not a case block
				}
				rows = append(rows, row)
			}
			found[path] = rows
		}
	}
	walk(doc, "")

	// 1. Closure, both directions.
	for path := range found {
		if _, ok := specs[path]; !ok {
			add("the block %q holds %d cases and `vocabulary.blocks` does not declare it — nothing says what those cases must carry, which is how sessionFilenames kept a reasonless refusal one block away from the check that forbids it [blocks-declared]",
				path, len(found[path]))
		}
	}
	for path := range specs {
		if _, ok := found[path]; !ok {
			add("`vocabulary.blocks` declares %q and no such array-of-objects block exists — a declaration for a renamed block validates nothing [blocks-exist]", path)
		}
	}

	for path, rows := range found {
		spec, ok := specs[path]
		if !ok {
			continue
		}
		// 2. Required fields, and unique case names.
		names := map[string]int{}
		for i, row := range rows {
			label := fmt.Sprintf("%s[%d]", path, i)
			if n, ok := row["name"].(string); ok && n != "" {
				label = fmt.Sprintf("%s %q", path, n)
				names[n]++
			}
			for _, field := range spec.Required {
				// PRESENCE, not truthiness. `"in": ""` and `"expect": null` are
				// real values a case may declare — what this check asks is
				// whether the case STATED the field, because an omitted verdict
				// or an omitted input is a case that says nothing.
				if _, ok := row[field]; !ok {
					add("%s has no %q, which vocabulary.blocks says every case in this block must carry [required-fields]", label, field)
				}
			}
			if spec.VerdictFld == "" {
				continue
			}
			verdict, _ := row[spec.VerdictFld].(string)
			def, known := spec.Verdicts[verdict]
			if !known {
				add("%s has %s %q, which is not one of the declared verdicts %v — an unknown verdict is a case every loader decides for itself [verdict-vocabulary]",
					label, spec.VerdictFld, verdict, sortedKeysOfVerdicts(spec.Verdicts))
				continue
			}
			for _, field := range def.Requires {
				if v, ok := row[field]; !ok || v == nil || v == "" {
					add("%s is a %q and names no %q — a bare verdict is satisfied by ANY outcome of that class, including one produced for a completely different reason [verdict-reason-required]",
						label, verdict, field)
					continue
				}
				if def.Reasons == "" {
					continue
				}
				declared, _ := vocab[def.Reasons].(map[string]any)
				if len(declared) == 0 {
					add("vocabulary.%s is missing or empty, but %s points its reasons at it [verdict-reason-declared]", def.Reasons, path)
					continue
				}
				reason, _ := row[field].(string)
				if _, ok := declared[reason]; !ok {
					add("%s claims %s %q, which vocabulary.%s does not declare [verdict-reason-declared]", label, field, reason, def.Reasons)
				}
			}
			for _, field := range def.Forbids {
				if v, ok := row[field]; ok && v != nil && v != "" {
					add("%s is a %q and carries %q, which only the other verdict may [verdict-reason-forbidden]", label, verdict, field)
				}
			}
		}
		for n, count := range names {
			if count > 1 {
				add("%s has %d cases named %q — a duplicate name hides one of them in every report [unique-case-names]", path, count, n)
			}
		}

		// 2b. FIELD CLOSURE, both directions.
		allowed := map[string]bool{}
		for _, f := range spec.Required {
			allowed[f] = true
		}
		for _, f := range spec.Optional {
			allowed[f] = true
		}
		usedOptional := map[string]bool{}
		usedNested := map[string]map[string]bool{}
		for i, row := range rows {
			label := fmt.Sprintf("%s[%d]", path, i)
			if n, ok := row["name"].(string); ok && n != "" {
				label = fmt.Sprintf("%s %q", path, n)
			}
			for key, val := range row {
				if !allowed[key] {
					add("%s carries %q, which vocabulary.blocks declares neither required nor optional — an undeclared key is ignored by encoding/json AND by JSON.parse, so a one-character typo in a field name silently defangs the case in every loader at once [unknown-fields]", label, key)
					continue
				}
				usedOptional[key] = true
				sub, declared := spec.Nested[key]
				if !declared {
					continue
				}
				obj, ok := val.(map[string]any)
				if !ok {
					continue
				}
				ok2 := map[string]bool{}
				for _, f := range sub {
					ok2[f] = true
				}
				for k := range obj {
					if !ok2[k] {
						add("%s has %s.%s, which vocabulary.blocks.%s.nested.%s does not declare — the same silent-typo defect one level down [unknown-fields]", label, key, k, path, key)
						continue
					}
					if usedNested[key] == nil {
						usedNested[key] = map[string]bool{}
					}
					usedNested[key][k] = true
				}
			}
		}
		for _, f := range spec.Optional {
			if !usedOptional[f] {
				add("vocabulary.blocks.%s declares the optional field %q and no case carries it — an optional field nothing uses legalizes a typo instead of catching one [optional-used]", path, f)
			}
		}
		for key, sub := range spec.Nested {
			for _, f := range sub {
				if !usedNested[key][f] {
					add("vocabulary.blocks.%s.nested.%s declares %q and no case carries it [optional-used]", path, key, f)
				}
			}
		}

		// 2c. Every block names the tests that READ it.
		if len(spec.Loaders) == 0 {
			add("vocabulary.blocks.%s names no `loaders` — the fixture-level loader count is per FILE, so this block could lose every test that reads it while the file kept the others and nothing would go red [block-loaders]", path)
		}
		for _, l := range spec.Loaders {
			if !strings.Contains(l, "::") {
				add("vocabulary.blocks.%s loader %q is not \"<repo-relative file>::<needle>\" — without a needle the entry only says a file exists, not that anything in it reads this block [block-loaders]", path, l)
			}
		}

		// 3. Every declared reason is EXERCISED. An unused arm of a
		// classification is one nothing holds the copies to.
		for verdict, def := range spec.Verdicts {
			if def.Reasons == "" {
				continue
			}
			declared, _ := vocab[def.Reasons].(map[string]any)
			used := map[string]bool{}
			for _, row := range rows {
				if got, _ := row[spec.VerdictFld].(string); got != verdict {
					continue
				}
				for _, field := range def.Requires {
					if r, ok := row[field].(string); ok {
						used[r] = true
					}
				}
			}
			for reason := range declared {
				if !used[reason] {
					add("vocabulary.%s declares %q and no %s case in %s names it — an unexercised classification arm is one nothing holds the copies to [reason-vocabulary-used]",
						def.Reasons, reason, verdict, path)
				}
			}
		}
	}

	// 4. Token references. A `${TOKEN}` that is not declared passes through
	// verbatim and silently defangs whatever uses it; this ran for
	// path-containment only, and a stray token in any other fixture was free.
	tokens, _ := vocab["tokens"].(map[string]any)
	walkStrings(doc, "", func(where, s string) {
		names, unterminated := contractTokenRefs(s)
		if unterminated {
			problems = append(problems, fmt.Sprintf(`%s: %s has a "${" with no closing "}" in %q — the substituter leaves it verbatim, exactly like a mis-spelled name [token-references]`, name, where, s))
		}
		for _, tok := range names {
			if _, ok := tokens[tok]; !ok {
				problems = append(problems, fmt.Sprintf("%s: %s references ${%s}, which vocabulary.tokens does not declare — it passes through verbatim [token-references]\n  in: %q", name, where, tok, s))
			}
		}
	})

	sort.Strings(problems)
	return problems
}

func sortedKeysOfVerdicts(m map[string]vocabVerdictDef) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

// walkStrings visits every string in a decoded document, keys included.
func walkStrings(v any, where string, visit func(where, s string)) {
	switch t := v.(type) {
	case string:
		visit(where, t)
	case []any:
		for i, e := range t {
			walkStrings(e, fmt.Sprintf("%s[%d]", where, i), visit)
		}
	case map[string]any:
		for k, e := range t {
			visit(where+"."+k+" (key)", k)
			walkStrings(e, where+"."+k, visit)
		}
	}
}

// contractsFixtures returns every fixture in contracts/, decoded.
func contractsFixtures(t *testing.T) map[string]map[string]any {
	t.Helper()
	dir, err := sweepguard.RepoPath("contracts")
	if err != nil {
		t.Fatalf("locate contracts/: %v", err)
	}
	// extinput.ReadDir: the LISTING is this guard's input — it validates every
	// fixture in contracts/ — and an out-of-module directory read with
	// os.ReadDir is not in cmd/go's test cache key, so a NEW fixture with an
	// undeclared block would arrive to `ok (cached)`. The per-file reads below
	// already go through sweepguard; this closes the enumeration.
	entries, err := extinput.ReadDir(dir)
	if err != nil {
		t.Fatalf("read contracts/: %v", err)
	}
	out := map[string]map[string]any{}
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		raw, err := sweepguard.ReadRepoFile("contracts", e.Name())
		if err != nil {
			t.Fatalf("read %s: %v", e.Name(), err)
		}
		var doc map[string]any
		if err := json.Unmarshal(raw, &doc); err != nil {
			t.Fatalf("parse %s: %v", e.Name(), err)
		}
		out[e.Name()] = doc
	}
	if len(out) == 0 {
		t.Fatal("contracts/ decoded to no fixtures at all")
	}
	return out
}

// TestEveryFixtureBlockIsDeclaredAndClosed is the guard itself.
func TestEveryFixtureBlockIsDeclaredAndClosed(t *testing.T) {
	fixtures := contractsFixtures(t)
	var tally sweepguard.Tally
	for name, doc := range fixtures {
		t.Run(name, func(t *testing.T) {
			// A few fixtures carry no case blocks at all (a JSON schema, a lock
			// file, a single job spec); they are declared exempt HERE rather
			// than skipped silently, so adding cases to one of them fails until
			// it is registered.
			if vocabExempt[name] {
				if _, ok := doc["vocabulary"]; ok {
					t.Fatalf("%s is on the no-blocks exemption list but now declares a vocabulary — take it off the list", name)
				}
				tally.Skip("declared exempt: carries no case blocks")
				t.Skipf("%s carries no case blocks", name)
			}
			tally.Ran("other")
			for _, p := range validateFixtureVocabulary(name, doc) {
				t.Error(p)
			}
		})
	}
	if err := tally.RequireEvery("the contracts vocabulary sweep", contractsFixtureFloor); err != nil {
		t.Fatal(err)
	}
	t.Log(tally.String())
}

// vocabExempt names the files in contracts/ that are not case corpora. It is an
// allow-list and not a heuristic: "has no blocks, so skip it" is how a fixture
// whose cases were deleted would pass.
//
// wholesale-config-paths.json came OFF this list when it grew `valueCases` (the
// guard forbids being on it while declaring a vocabulary, which is what made
// that a required edit rather than an optional one).
var vocabExempt = map[string]bool{
	"session-schema.json":        true,
	"config-lock.json":           true,
	"job-preset-power-down.json": true,
}

// contractsFixtureFloor is how many case-carrying fixtures contracts/ holds.
const contractsFixtureFloor = 10

// TestTheVocabularyGuardIsFalsifiable is the answer to "all three vocabulary
// tests are neuterable with every suite green". Each mutation below is a real
// defect this validator claims to catch; the battery runs the SAME function the
// test above runs, over a copy of the real fixture with one thing broken, and
// requires the matching check ID to fire. Delete a check and this goes red.
func TestTheVocabularyGuardIsFalsifiable(t *testing.T) {
	base := contractsFixtures(t)["path-containment-cases.json"]
	if base == nil {
		t.Fatal("the containment fixture is not in contracts/")
	}

	for _, tc := range []struct {
		name   string
		checkD string
		mutate func(doc map[string]any)
	}{
		{"a deny case loses its reason", "verdict-reason-required", func(doc map[string]any) {
			for _, row := range casesOf(doc, "cases") {
				if row["expect"] == "deny" {
					delete(row, "deniedBy")
					return
				}
			}
		}},
		{"a refuse case loses its reason", "verdict-reason-required", func(doc map[string]any) {
			for _, row := range casesOf(doc, "sessionFilenames", "cases") {
				if row["expect"] == "refuse" {
					delete(row, "refusedBy")
					return
				}
			}
		}},
		{"a deny case claims an undeclared reason", "verdict-reason-declared", func(doc map[string]any) {
			for _, row := range casesOf(doc, "cases") {
				if row["expect"] == "deny" {
					row["deniedBy"] = "outside-rooots"
					return
				}
			}
		}},
		{"a case invents a verdict word", "verdict-vocabulary", func(doc map[string]any) {
			casesOf(doc, "cases")[0]["expect"] = "maybe"
		}},
		{"an allow case carries a deny's reason", "verdict-reason-forbidden", func(doc map[string]any) {
			for _, row := range casesOf(doc, "cases") {
				if row["expect"] == "allow" {
					row["deniedBy"] = "secret"
					return
				}
			}
		}},
		{"a case drops a required field", "required-fields", func(doc map[string]any) {
			delete(casesOf(doc, "spawnCwds", "cases")[0], "why")
		}},
		{"a new block appears with no declaration", "blocks-declared", func(doc map[string]any) {
			doc["newIdeas"] = []any{map[string]any{"name": "x"}}
		}},
		{"a declared block is renamed away", "blocks-exist", func(doc map[string]any) {
			doc["checkUseRenamed"] = doc["checkUse"]
			delete(doc, "checkUse")
		}},
		{"a declared reason stops being exercised", "reason-vocabulary-used", func(doc map[string]any) {
			for _, row := range casesOf(doc, "sessionFilenames", "cases") {
				if row["refusedBy"] == "escapes-sessions-dir" {
					row["refusedBy"] = "not-a-basename"
				}
			}
		}},
		{"two cases share a name", "unique-case-names", func(doc map[string]any) {
			rows := casesOf(doc, "cases")
			rows[1]["name"] = rows[0]["name"]
		}},
		{"a token is mis-spelled", "token-references", func(doc map[string]any) {
			casesOf(doc, "cases")[0]["target"] = "${ROOOT}/x"
		}},
		{"a token reference is unterminated", "token-references", func(doc map[string]any) {
			casesOf(doc, "cases")[0]["target"] = "${ROOT/x"
		}},
		{"a case field name is mis-spelled by one character", "unknown-fields", func(doc map[string]any) {
			for _, row := range casesOf(doc, "cases") {
				if v, ok := row["configDirVia"]; ok {
					delete(row, "configDirVia")
					row["configDirVla"] = v
					return
				}
			}
		}},
		{"a tree sub-key is mis-spelled", "unknown-fields", func(doc map[string]any) {
			for _, row := range casesOf(doc, "cases") {
				tree, _ := row["tree"].(map[string]any)
				if v, ok := tree["symlinks"]; ok {
					delete(tree, "symlinks")
					tree["symLinks"] = v
					return
				}
			}
		}},
		{"an optional field is declared and used by nothing", "optional-used", func(doc map[string]any) {
			spec := doc["vocabulary"].(map[string]any)["blocks"].(map[string]any)["cases"].(map[string]any)
			spec["optional"] = append(spec["optional"].([]any), "configDirVla")
		}},
		{"a block stops naming the tests that read it", "block-loaders", func(doc map[string]any) {
			spec := doc["vocabulary"].(map[string]any)["blocks"].(map[string]any)["sessionFilenames.cases"].(map[string]any)
			delete(spec, "loaders")
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			doc := deepCopyDoc(t, base)
			if problems := validateFixtureVocabulary("mutant", doc); len(problems) != 0 {
				t.Fatalf("the UNMUTATED copy already reports problems, so this case proves nothing:\n  %s", strings.Join(problems, "\n  "))
			}
			tc.mutate(doc)
			problems := validateFixtureVocabulary("mutant", doc)
			if len(problems) == 0 {
				t.Fatalf("mutation %q produced NO complaint — the %s check does not bite, and every fixture in contracts/ is free to carry that defect", tc.name, tc.checkD)
			}
			hit := false
			for _, p := range problems {
				if strings.Contains(p, "["+tc.checkD+"]") {
					hit = true
				}
			}
			if !hit {
				t.Fatalf("mutation %q was caught by something, but not by %s:\n  %s", tc.name, tc.checkD, strings.Join(problems, "\n  "))
			}
		})
	}
}

func casesOf(doc map[string]any, path ...string) []map[string]any {
	var cur any = doc
	for _, p := range path {
		cur = cur.(map[string]any)[p]
	}
	list := cur.([]any)
	out := make([]map[string]any, 0, len(list))
	for _, e := range list {
		out = append(out, e.(map[string]any))
	}
	return out
}

func deepCopyDoc(t *testing.T, doc map[string]any) map[string]any {
	t.Helper()
	raw, err := json.Marshal(doc)
	if err != nil {
		t.Fatal(err)
	}
	var out map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		t.Fatal(err)
	}
	return out
}

// TestBothCorpusVocabularyLoadersExist is the CROSS-LOADER EXISTENCE GUARD.
//
// The three copies of the containment vocabulary test agreed only because a
// comment said "TWINS: ...". Deleting one of them, or quietly dropping a check
// from one, was a green run everywhere: no test read another loader's source.
// This one does — for this validator and for the three per-loader vocabulary
// tests — so a check that exists on one side and not the other is named.
func TestBothCorpusVocabularyLoadersExist(t *testing.T) {
	tsPath := filepath.Join("apps", "desktop", "src", "main", "services", "contractsVocabulary.test.ts")
	ts, err := sweepguard.ReadRepoFile(strings.Split(filepath.ToSlash(tsPath), "/")...)
	if err != nil {
		t.Fatalf("the TypeScript twin of this validator is unreadable: %v", err)
	}
	for _, id := range vocabCheckIDs {
		if !strings.Contains(string(ts), "["+id+"]") {
			t.Errorf("%s does not carry the %q check — the two validators have drifted, and a fixture defect this side catches would ship on the other", tsPath, id)
		}
	}

	// And the three per-loader vocabulary tests, which are the OTHER copies.
	for _, twin := range []struct {
		path  string
		needs []string
	}{
		{filepath.Join("services", "hub", "cmd", "brain", "fsguard_test.go"), []string{"TestFixtureVocabularyIsClosed"}},
		{filepath.Join("services", "hub", "internal", "bus", "policy_test.go"), []string{"TestFixtureVocabularyIsClosed"}},
		{filepath.Join("apps", "desktop", "src", "main", "lib", "pathConfinement.test.ts"), []string{"describe('the fixture vocabulary is closed'"}},
	} {
		src, err := sweepguard.ReadRepoFile(strings.Split(filepath.ToSlash(twin.path), "/")...)
		if err != nil {
			t.Errorf("%s is unreadable, so its copy of the vocabulary check is gone: %v", twin.path, err)
			continue
		}
		for _, need := range twin.needs {
			if !strings.Contains(string(src), need) {
				t.Errorf("%s no longer contains %q — one of the three vocabulary loaders has been removed and the other two would not notice", twin.path, need)
			}
		}
	}
}
