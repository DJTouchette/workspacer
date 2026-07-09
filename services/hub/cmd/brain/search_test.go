package main

import (
	"path/filepath"
	"testing"
)

// TestParseRipgrepJSONMultipleSubmatchesPerLine covers a single line containing
// several occurrences of the query. rg emits one "match" message with multiple
// submatches; the parser must surface each as its own result (distinct column),
// not collapse them to the first submatch.
func TestParseRipgrepJSONMultipleSubmatchesPerLine(t *testing.T) {
	out := `{"type":"begin","data":{"path":{"text":"a.txt"}}}
{"type":"match","data":{"path":{"text":"a.txt"},"lines":{"text":"foo foo foo\n"},"line_number":7,"submatches":[{"start":0},{"start":4},{"start":8}]}}
{"type":"end","data":{"path":{"text":"a.txt"}}}`

	res := parseRipgrepJSON([]byte(out), "/proj", 500)
	if len(res.Results) != 1 {
		t.Fatalf("expected 1 file, got %d", len(res.Results))
	}
	got := res.Results[0]
	if got.File != filepath.Join("/proj", "a.txt") {
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

// Truncation must count individual submatches, not lines.
func TestParseRipgrepJSONTruncatesBySubmatch(t *testing.T) {
	out := `{"type":"match","data":{"path":{"text":"a.txt"},"lines":{"text":"x x x\n"},"line_number":1,"submatches":[{"start":0},{"start":2},{"start":4}]}}`
	res := parseRipgrepJSON([]byte(out), "/proj", 2)
	if !res.Truncated {
		t.Error("expected Truncated when submatches exceed maxResults")
	}
	if n := len(res.Results[0].Matches); n != 2 {
		t.Fatalf("expected 2 matches under the cap, got %d", n)
	}
}
