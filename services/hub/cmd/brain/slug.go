package main

// Filename slugs for layouts and saved sessions — a Go port of fileUtils.ts
// `slug()` (the 'layout' charset variant) so the brain writes and deletes the
// exact same filenames the app does. Idempotent: slug(slug(x)) == slug(x), which
// matters because remove() re-slugs a stored id.

import (
	"regexp"
	"strings"
)

var (
	reSlugBad    = regexp.MustCompile(`[^a-z0-9_-]`)
	reSlugDashes = regexp.MustCompile(`-+`)
	reLibBad     = regexp.MustCompile(`[^a-z0-9_-]+`)
)

// lowerASCII folds A-Z and leaves every other byte alone.
//
// strings.ToLower is NOT interchangeable with JavaScript's toLowerCase, and this
// is the one place in the repo where the difference decides a FILENAME. Go does
// a per-rune simple fold; JS does the full Unicode SPECIAL CASING map, which can
// make a string longer. U+0130 (İ) folds to a single 'i' in Go and to
// 'i' + U+0307 COMBINING DOT ABOVE in JS — where the combining mark is then a
// bad character and becomes a '-'. So "aİb" was written as aib.yaml by this
// brain and as ai-b.yaml by the app, into the same store: the item was invisible
// to the other provider's list, and delete re-slugged and unlinked a filename
// that was never written, so it could not be removed either.
//
// Every character the two implementations can disagree about is non-ASCII, and
// every non-ASCII character is replaced by '-' below anyway, so narrowing the
// fold to ASCII costs nothing a caller could want and removes the whole class —
// including the cases nobody has enumerated yet. The twin is fileUtils.ts
// `lowerAscii`; contracts/filename-slug-cases.json holds both to it.
func lowerASCII(s string) string {
	b := []byte(s)
	for i := 0; i < len(b); i++ {
		if b[i] >= 'A' && b[i] <= 'Z' {
			b[i] += 'a' - 'A'
		}
	}
	return string(b)
}

func slugify(name string, trimDashes bool, maxLen int, fallback string) string {
	out := lowerASCII(name)
	out = reSlugBad.ReplaceAllString(out, "-")
	out = reSlugDashes.ReplaceAllString(out, "-")
	if trimDashes {
		out = strings.Trim(out, "-")
	}
	if maxLen > 0 && len(out) > maxLen {
		out = out[:maxLen]
	}
	if trimDashes {
		out = strings.Trim(out, "-")
	}
	if out == "" && fallback != "" {
		return fallback
	}
	return out
}

// slugLayout matches fileUtils.slugLayout (trim dashes, max 64, fallback 'layout').
func slugLayout(name string) string { return slugify(name, true, 64, "layout") }

// slugSession matches fileUtils.slugSession (no trim, max 64, no fallback).
func slugSession(name string) string { return slugify(name, false, 64, "") }

// slugLibrary matches fileUtils.slugLibrary: collapse runs of bad chars into one
// '-', trim all leading/trailing dashes, fallback 'item', no length cap.
func slugLibrary(s string) string {
	out := reLibBad.ReplaceAllString(lowerASCII(s), "-")
	out = strings.Trim(out, "-")
	if out == "" {
		return "item"
	}
	return out
}
