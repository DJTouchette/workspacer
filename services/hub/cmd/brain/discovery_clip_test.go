package main

import (
	"strings"
	"testing"
	"unicode/utf8"
)

// clip must not split a multibyte UTF-8 rune. A summary whose 100th byte lands
// in the middle of a multibyte character (accents, CJK, emoji) previously came
// back as invalid UTF-8, which JSON-marshals to a mangled replacement char in
// the resume picker.
func TestClipDoesNotSplitRune(t *testing.T) {
	// 99 ASCII bytes + "é" (2 bytes: 0xC3 0xA9) => byte offset 100 is mid-rune.
	s := strings.Repeat("a", 99) + "é"
	got := clip(s, 100)
	if !utf8.ValidString(got) {
		t.Fatalf("clip returned invalid UTF-8: %q (bytes %v)", got, []byte(got))
	}
	// It should keep the 99 a's and the whole final rune (rune count 100 -> keep 100 runes).
	if got != strings.Repeat("a", 99)+"é" {
		t.Fatalf("unexpected clip result: %q", got)
	}
}

// The astral half, and the one the desktop twin failed. Go counts RUNES here and
// TypeScript's String.slice counts UTF-16 CODE UNITS, so every non-BMP character
// costs two on that side: the same summary came back at 100 code points from the
// brain and 50 from the desktop, and an odd boundary left a LONE LEAD SURROGATE
// that JSON.stringify emits as a bare \ud83d. claude.sessionsForDir is answered
// by whichever provider is registered, so the two clips have to be one clip.
// TWIN: clip() in apps/desktop/src/main/services/claudeSessionList.ts, pinned by
// claudeSessionList.test.ts with these same two vectors.
func TestClipCountsCodePointsNotUTF16Units(t *testing.T) {
	// An odd boundary: one ASCII byte then 150 astral characters. A UTF-16
	// counter stops mid-surrogate-pair at unit 100.
	odd := "a" + strings.Repeat("\U0001F600", 150)
	got := clip(odd, 100)
	if n := utf8.RuneCountInString(got); n != 100 {
		t.Fatalf("clip kept %d code points, want 100 (a UTF-16 counter keeps 51)", n)
	}
	if !utf8.ValidString(got) {
		t.Fatalf("clip returned invalid UTF-8: %q", got)
	}
	if !strings.HasSuffix(got, "\U0001F600") {
		t.Fatalf("clip split the final character: %q", got)
	}
	// An even boundary: a UTF-16 counter keeps 50 whole characters here, which is
	// valid text and therefore invisible without counting.
	even := strings.Repeat("\U0001F600", 150)
	if n := utf8.RuneCountInString(clip(even, 100)); n != 100 {
		t.Fatalf("clip kept %d code points of an all-astral summary, want 100", n)
	}
}
