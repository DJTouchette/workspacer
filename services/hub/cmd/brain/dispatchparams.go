package main

import (
	"regexp"
	"strings"
)

// The placeholder half of a dispatch template, headless.
//
// TWIN: apps/desktop/src/main/lib/dispatchTemplate.ts `dispatchTemplateParams`,
// held equal by contracts/dispatch-template-params-cases.json.
//
// WHY THIS SIDE HAS A COPY AT ALL, given the brain declines `template` on
// agents.spawn (parity_test.go spawnParamsDeclined): the brain IS the provider
// for `library.list` under the default catalog delegation, so it is the copy a
// web/mobile/MCP manager's pre-spawn discovery actually reaches. Serving the
// listing without `params` there would have made the SAME call answer
// differently depending on which provider ran — the exact divergence class
// provider-parity-cases.json exists to kill. Parsing is not rendering: this side
// advertises what a template wants and still refuses to render one.
//
// dispatchParam is the advertised shape. `Required` is emitted unconditionally
// (no omitempty) so a required param is a visible `"required": true` and an
// optional one a visible `false`, rather than an absent key a reader has to
// interpret.
type dispatchParam struct {
	Name     string `json:"name"`
	Required bool   `json:"required"`
	// Default is the template author's explicit default, present only when
	// !Required. omitempty is right here and wrong above: a required param has
	// no default, and an optional one whose default is the empty string renders
	// as empty either way.
	Default string `json:"default,omitempty"`
}

// jsWhitespace is ECMAScript's WhiteSpace ∪ LineTerminator, written out.
//
// It is spelled explicitly because NEITHER of Go's obvious spellings matches
// what the TS twin does, and the twin is the definition — the desktop provider
// answers the same `library.list` call. Go's regexp `\s` is only [\t\n\f\r ]
// (no \v, no NBSP), while `strings.TrimSpace`/`unicode.IsSpace` is wider than
// JS in one direction (U+0085 NEL) and narrower in another (U+FEFF BOM). Either
// one would make a template whose token is padded with an exotic space parse to
// a different param name on the two providers — a name the caller then cannot
// fill, on one provider only.
const jsWhitespace = "\t\n\v\f\r \u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005" +
	"\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"

// dispatchTokenRe mirrors dispatchTemplate.ts's TOKEN_RE, with `\s` expanded to
// the class above.
var dispatchTokenRe = regexp.MustCompile(`\{\{[` + jsWhitespace + `]*([^}]+?)[` + jsWhitespace + `]*\}\}`)

// trimJS is JavaScript's String.prototype.trim(), exactly. See jsWhitespace.
func trimJS(s string) string { return strings.Trim(s, jsWhitespace) }

// dispatchAutoVars are filled by the HOST from the spawn's own context, not by
// the caller's templateParams, so they are excluded from the advertised list —
// what `params` describes is what a CALLER must or may pass. TWIN: AUTO_VARS.
var dispatchAutoVars = map[string]bool{"cwd": true}

// parseDispatchPlaceholder splits one token's inner text ("task", "?task",
// "delivery:open a PR"). The leading '?' is the renderer's prompt-var spelling
// and is tolerated so a template authored for the insert dialog parses here
// unchanged; only the FIRST ':' splits, so a default may contain colons.
func parseDispatchPlaceholder(inner string) (name, def string, optional bool) {
	rest := strings.TrimPrefix(inner, "?")
	ci := strings.Index(rest, ":")
	if ci < 0 {
		return trimJS(rest), "", false
	}
	return trimJS(rest[:ci]), trimJS(rest[ci+1:]), true
}

// dispatchTemplateParams lists the distinct placeholders a dispatch template
// declares, auto vars excluded, FIRST OCCURRENCE WINNING — so a name spelled
// bare once and with a default once is advertised as required, which is what the
// desktop renderer then enforces token by token.
func dispatchTemplateParams(text string) []dispatchParam {
	var out []dispatchParam
	seen := map[string]bool{}
	for _, m := range dispatchTokenRe.FindAllStringSubmatch(text, -1) {
		name, def, optional := parseDispatchPlaceholder(trimJS(m[1]))
		if name == "" || dispatchAutoVars[name] || seen[name] {
			continue
		}
		seen[name] = true
		out = append(out, dispatchParam{Name: name, Required: !optional, Default: def})
	}
	return out
}
