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
