---
title: Porting a JS regex/trim to Go: neither \s nor TrimSpace is JavaScript's whitespace
date: 2026-08-26
confidence: high
related_paths:
  - apps/desktop/src/main/lib/dispatchTemplate.ts
  - services/hub/cmd/brain/dispatchparams.go
  - contracts/dispatch-template-params-cases.json
  - apps/desktop/src/main/shared/libraryKinds.ts
promoted: true
promoted_to: agent-spawn
---

# Porting a JS regex/trim to Go: neither \s nor TrimSpace is JavaScript's whitespace

## Observation
The dispatch-template placeholder parser (apps/desktop/src/main/lib/dispatchTemplate.ts) spells its token with JavaScript's `\s` and trims with String.prototype.trim(). NEITHER Go spelling matches: Go's regexp `\s` is only [\t\n\f\r ] (no \v U+000B, no NBSP U+00A0), while strings.TrimSpace/unicode.IsSpace is WIDER in one direction (trims U+0085 NEL, which JS does not) and NARROWER in another (does not trim U+FEFF BOM, which JS does). services/hub/cmd/brain/dispatchparams.go therefore writes the ECMAScript WhiteSpace ∪ LineTerminator set out by hand (`jsWhitespace`) and uses it for both the character class and strings.Trim. Four cases in contracts/dispatch-template-params-cases.json pin all four divergent code points.

Two other seam facts from the same change: (1) LIBRARY_KINDS had to live in apps/desktop/src/main/shared/libraryKinds.ts rather than libraryService.ts, because SIX suites do `vi.mock('./libraryService', ...)` wholesale and a value imported from there is undefined under the mock; (2) hubCapabilitiesKillSwitch.test.ts extracted the per-file guard positionally as `mock.calls[0].at(-1)`, which silently stopped meaning "the guard" the moment library.list grew a trailing filter argument — it now finds the single function-typed argument (`guardArgOf`) and asserts there is exactly one.</observation>
<parameter name="impact">A TrimSpace-based port would have made the same library.list call advertise a param name the caller cannot fill, on one provider only — invisible until a template author pasted a non-breaking space.

## Recommendation
When porting any JS string parser to Go, write the whitespace class out explicitly and pin the four divergent code points (U+000B, U+00A0, U+0085, U+FEFF) in a contract fixture.
