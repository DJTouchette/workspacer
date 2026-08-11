package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"unicode/utf8"
)

// TestParseRipgrepJSONMultipleSubmatchesPerLine covers a single line containing
// several occurrences of the query. rg emits one "match" message with multiple
// submatches; the parser must surface each as its own result (distinct column),
// not collapse them to the first submatch.
func TestParseRipgrepJSONMultipleSubmatchesPerLine(t *testing.T) {
	out := `{"type":"begin","data":{"path":{"text":"a.txt"}}}
{"type":"match","data":{"path":{"text":"a.txt"},"lines":{"text":"foo foo foo\n"},"line_number":7,"submatches":[{"start":0},{"start":4},{"start":8}]}}
{"type":"end","data":{"path":{"text":"a.txt"}}}`

	res := parseRipgrepJSON([]byte(out), absTestPath("proj"), 500)
	if len(res.Results) != 1 {
		t.Fatalf("expected 1 file, got %d", len(res.Results))
	}
	got := res.Results[0]
	if got.File != filepath.Join(absTestPath("proj"), "a.txt") {
		t.Errorf("file = %q", got.File)
	}
	if len(got.Matches) != 3 {
		t.Fatalf("expected 3 matches (one per submatch), got %d: %+v", len(got.Matches), got.Matches)
	}
	wantCols := []int{1, 5, 9}
	for i, m := range got.Matches {
		if m.Line != 7 {
			t.Errorf("match %d line = %d, want 7", i, m.Line)
		}
		if m.Column != wantCols[i] {
			t.Errorf("match %d column = %d, want %d", i, m.Column, wantCols[i])
		}
	}
}

// The `text` field is the other half of search.project's shape, and the two
// providers disagreed on both of its transformations.
//
// CLIP. Go counts RUNES (clip → utf8.RuneCountInString) and TypeScript's
// String.slice counts UTF-16 CODE UNITS, so every non-BMP character costs two
// there: a matching line of astral characters came back at 300 code points from
// this provider and 153 from the desktop, and an ODD boundary left a lone lead
// surrogate — JSON.stringify emits it as a bare \ud83d, which Go's
// json.Unmarshal turns into U+FFFD and a strict JSON reader rejects outright.
// searchService.ts now uses the same code-point clip claude.sessionsForDir does.
//
// TRIM. strings.TrimSpace and JS `.trim()` disagree on U+FEFF and U+0085, so a
// matching line beginning with a BOM came back with different text from each.
// Both copies now strip the ASCII whitespace set and nothing else.
//
// TWIN: searchService.astralText.test.ts, with these same vectors.
func TestParseRipgrepJSONClipsCodePointsAndTrimsAsciiOnly(t *testing.T) {
	const emoji = "\U0001F600"
	// An ODD boundary: one ASCII byte then astral characters, so a UTF-16
	// counter stops mid-surrogate-pair.
	odd := "a" + strings.Repeat(emoji, 400)
	even := strings.Repeat(emoji, 400)
	// U+FEFF leads, U+0085 trails: the two code points the built-in trims
	// disagree about, one in each direction.
	bom := "\ufeffNEEDLE\u0085"

	out := strings.Join([]string{
		`{"type":"match","data":{"path":{"text":"odd.txt"},"lines":{"text":` + jsonStr(odd+"\n") + `},"line_number":1,"submatches":[{"start":0}]}}`,
		`{"type":"match","data":{"path":{"text":"even.txt"},"lines":{"text":` + jsonStr(even+"\n") + `},"line_number":1,"submatches":[{"start":0}]}}`,
		`{"type":"match","data":{"path":{"text":"bom.txt"},"lines":{"text":` + jsonStr(bom+"\n") + `},"line_number":1,"submatches":[{"start":0}]}}`,
	}, "\n")

	res := parseRipgrepJSON([]byte(out), absTestPath("proj"), 500)
	byFile := map[string]string{}
	for _, f := range res.Results {
		byFile[filepath.Base(f.File)] = f.Matches[0].Text
	}
	for _, name := range []string{"odd.txt", "even.txt"} {
		got := byFile[name]
		if n := utf8.RuneCountInString(got); n != searchMaxTextLen {
			t.Errorf("%s: kept %d code points, want %d (a UTF-16 counter keeps ~150)", name, n, searchMaxTextLen)
		}
		if !utf8.ValidString(got) {
			t.Errorf("%s: text is not valid UTF-8 — a boundary split a character", name)
		}
		if !strings.HasSuffix(got, emoji) {
			t.Errorf("%s: the final character was split: %q", name, got[len(got)-8:])
		}
	}
	if got := byFile["bom.txt"]; got != bom {
		t.Errorf("bom.txt: text = %q, want %q — U+FEFF and U+0085 are ordinary characters here, "+
			"and only the ASCII whitespace set is trimmed", got, bom)
	}
}

// A non-positive cap means "unset", not "return nothing". `??` in the desktop
// twin only replaced null/undefined, so maxResults:0 was a literal cap of zero
// there — an empty result list flagged truncated:true — and the full 500 here.
func TestEffectiveMaxResultsTreatsNonPositiveAsUnset(t *testing.T) {
	for _, requested := range []int{0, -1, -500} {
		if got := effectiveMaxResults(requested); got != searchMaxResults {
			t.Errorf("effectiveMaxResults(%d) = %d, want the default %d", requested, got, searchMaxResults)
		}
	}
	if got := effectiveMaxResults(7); got != 7 {
		t.Errorf("effectiveMaxResults(7) = %d — a real cap must survive", got)
	}
}

// Truncation must count individual submatches, not lines.
func TestParseRipgrepJSONTruncatesBySubmatch(t *testing.T) {
	out := `{"type":"match","data":{"path":{"text":"a.txt"},"lines":{"text":"x x x\n"},"line_number":1,"submatches":[{"start":0},{"start":2},{"start":4}]}}`
	res := parseRipgrepJSON([]byte(out), absTestPath("proj"), 2)
	if !res.Truncated {
		t.Error("expected Truncated when submatches exceed maxResults")
	}
	if n := len(res.Results[0].Matches); n != 2 {
		t.Fatalf("expected 2 matches under the cap, got %d", n)
	}
}

// TestSearchDropsFilesFsReadWouldRefuse pins the read-set invariant: the set of
// files search.project may return CONTENT from cannot exceed fs.read's.
//
// The composition it closes takes two calls that are each correctly confined.
// search.project applies assertPathAllowed to its CWD and to nothing else, and
// delegates per-file exclusion to ripgrep's hidden/ignore walker — whose policy
// is a file INSIDE the searched directory. `<cwd>/.ignore` holding "!*" is an
// ordinary dotfile to every guard here: not a credential basename, no `.git`
// component, inside the root. So:
//
//	fs.write       <root>/.ignore   "!*\n!**/*\n"     -> allowed
//	search.project { cwd: <root> }                    -> allowed
//
// and the second returned matching lines out of `<root>/.git/config` and
// `<root>/.settings.json` — the two files the secret gate exists to refuse.
// Bytes written as DATA by the first call became the READ POLICY of the second.
//
// Driven at the collector rather than through `rg`, because the walker's policy
// is precisely what must stop mattering: this asserts the outcome for any walker
// that hands those paths back, including a future ripgrep whose ignore
// precedence differs.
//
// TWIN: searchService.readSet.test.ts.
func TestSearchDropsFilesFsReadWouldRefuse(t *testing.T) {
	sandboxHome(t)
	root := t.TempDir()
	root, err := filepath.EvalSymlinks(root)
	if err != nil {
		t.Fatal(err)
	}
	seed := map[string]string{
		filepath.Join(root, "readme.md"):                "ordinary CHAIN_SECRET_TOKEN placeholder\n",
		filepath.Join(root, ".git", "config"):           "url = https://x:CHAIN_SECRET_TOKEN@github.com/a/b.git\n",
		filepath.Join(root, ".settings.json"):           "{\"apiKey\":\"CHAIN_SECRET_TOKEN\"}\n",
		filepath.Join(root, ".claude", "settings.json"): "{\"hooks\":{}}\n",
		filepath.Join(root, ".mcp.json"):                "{}\n",
	}
	for p, body := range seed {
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	var lines []string
	rel := []string{"readme.md", ".git/config", ".settings.json", ".claude/settings.json", ".mcp.json"}
	for _, r := range rel {
		lines = append(lines, `{"type":"match","data":{"path":{"text":`+jsonStr(r)+
			`},"lines":{"text":"CHAIN_SECRET_TOKEN\n"},"line_number":1,"submatches":[{"start":0}]}}`)
	}
	res := parseRipgrepJSON([]byte(strings.Join(lines, "\n")), root, 500)

	got := map[string]bool{}
	for _, f := range res.Results {
		got[f.File] = true
	}
	for _, r := range rel[1:] {
		p := filepath.Join(root, filepath.FromSlash(r))
		if got[p] {
			t.Errorf("search.project returned bytes from %s — fs.read refuses that path, so a caller who plants an ignore file reads around the secret gate with two correctly-confined calls", p)
		}
	}
	// THE FLOOR. Without it a gate that drops every result passes the loop above
	// and the capability silently returns nothing at all.
	if !got[filepath.Join(root, "readme.md")] {
		t.Fatal("search.project dropped an ORDINARY file too — the per-file gate is refusing everything, which passes the deny loop above while breaking the capability")
	}
}
