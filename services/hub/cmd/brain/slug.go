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

func slugify(name string, trimDashes bool, maxLen int, fallback string) string {
	out := strings.ToLower(name)
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
	out := reLibBad.ReplaceAllString(strings.ToLower(s), "-")
	out = strings.Trim(out, "-")
	if out == "" {
		return "item"
	}
	return out
}
