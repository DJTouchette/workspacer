package main

// Compose a brief line from a worker's STRUCTURED RESULT instead of retyping it
// — the headless half of `brief.append`'s optional sessionId/result params.
//
// TWIN: apps/desktop/src/main/lib/briefResultLine.ts. That file carries the full
// argument; the short version is that a brief line welds a sentence of
// JUDGEMENT (the manager's, irreplaceable) to a run of MECHANICAL FACTS (the
// worker's, already reported verbatim in its wks-result block), and the manager
// was retyping both — spending tokens on the machine half and mistranscribing
// it. A live manager wrote `session:6a-round2` into a brief, a nickname where a
// session id belongs, and the dead link had to be repaired by hand.
//
// THIS IS PORTED RATHER THAN DECLINED, unlike agents.spawn's desktop-only params
// (see parity_test.go's spawnParamsDeclined). The reason is the shape of the
// work: composition here is a PURE STRING FUNCTION over the caller's own params.
// It needs no session store, no facade, no token mint, no worktree — nothing the
// headless brain lacks. A decline would have been an excuse rather than a
// reason, and it would have left a headless manager writing exactly the
// mistranscribed references this feature exists to eliminate.
//
// The two hard rules, and both are refusals:
//
//  1. THE SIGNIFICANCE SENTENCE IS REQUIRED. A result object alone can never
//     produce a line. Same rule dispatchTemplate.ts enforces on `{{task}}`, for
//     the same reason: the composed artifact READS FINISHED, so a caller who
//     could skip the judgement slot would ship machine exhaust that looks like
//     reasoning.
//  2. A MALFORMED SESSION ID IS REFUSED. Never guessed at, never passed through.
//     That is the entire error class this exists to kill.
//
// LOSSY BUT HONEST: long lists are capped and SAY SO ("+4 more"); caveats are
// capped by nothing at all, because a brief line that silently drops "the
// migration is not reversible" is worse than no line. If the caveats push the
// line past briefLineMax the whole write is refused, which is the correct loud
// failure.

import (
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
)

const (
	// factItemsShown is how many items of a capped list are rendered before the
	// "+K more" tail. TWIN: FACT_ITEMS_SHOWN.
	factItemsShown = 3
	// factValueMax is the longest a rendered fact VALUE may be before it is cut
	// with an announced ellipsis. Caveats are exempt. TWIN: FACT_VALUE_MAX.
	factValueMax = 200
	// sessionRefShortLen is the canonical short form the briefs and the UI link
	// on — a UUID's first group. TWIN: SESSION_REF_SHORT_LEN.
	sessionRefShortLen = 8
)

// factOrder is the common wks-result payload in the order a reader wants it:
// what landed, where, what proved it, what is still wrong, what is next. Keys
// NOT in this list are still rendered (the payload is caller-defined JSON and
// this must not become an allowlist that drops a field somebody chose to
// report) — they follow, sorted, since Go maps have no insertion order.
// TWIN: FACT_ORDER.
var factOrder = []string{"commit", "filesChanged", "checksRun", "caveats", "followUps"}

// uncappedFactKeys are rendered whole. See the header: a dropped caveat is the
// one loss this refuses to take. TWIN: UNCAPPED_KEYS.
var uncappedFactKeys = map[string]bool{"caveats": true, "caveat": true}

var (
	bareHexRe   = regexp.MustCompile(`^[0-9a-f]{6,}$`)
	uuidRe      = regexp.MustCompile(`^([0-9a-f]{8})-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)
	fullShaRe   = regexp.MustCompile(`^[0-9a-fA-F]{40}$`)
	leadDateRe  = regexp.MustCompile(`^\d{4}-\d{2}-\d{2}\b`)
	whitespaceR = regexp.MustCompile(`\s+`)
)

// errMalformedSessionRef is the refusal the whole feature turns on.
// TWIN: MalformedSessionRef.
func errMalformedSessionRef(given string) error {
	return fmt.Errorf("brief.append: %q is not a session id, so nothing was written. "+
		"A session reference is the worker's own id — a full UUID, or at least its first 6 "+
		"hex characters — and it is written into the brief as `session:<id>` so the user can "+
		"click through to that agent. A label, a round number, a nickname or a slug "+
		"(\"6a-round2\") is not a session id and would leave a dead link in the user's brief. "+
		"Copy the id from list_agents or from the wake that reported the result.", given)
}

// normalizeSessionRef validates a caller-supplied session id and returns the
// CANONICAL SHORT FORM. Tolerated on the way in: surrounding whitespace, a
// `session:` prefix typed out of habit, upper-case hex. Refused: anything that
// is not a hex run of 6+ or a full UUID. TWIN: normalizeSessionRef.
func normalizeSessionRef(raw string) (string, error) {
	text := strings.TrimSpace(raw)
	bare := strings.TrimSpace(strings.TrimPrefix(strings.TrimPrefix(text, "session:"), "SESSION:"))
	bare = strings.ToLower(bare)
	if bare == "" {
		return "", errMalformedSessionRef(text)
	}
	if m := uuidRe.FindStringSubmatch(bare); m != nil {
		return m[1], nil
	}
	if !bareHexRe.MatchString(bare) {
		return "", errMalformedSessionRef(text)
	}
	if len(bare) > sessionRefShortLen {
		return bare[:sessionRefShortLen], nil
	}
	return bare, nil
}

// scalarFactText flattens one JSON value to a single line. Objects and nested
// arrays become compact JSON rather than a Go %v rendering, so what lands in the
// brief is something a reader recognises. TWIN: scalarText.
func scalarFactText(v any) string {
	switch t := v.(type) {
	case string:
		return strings.TrimSpace(whitespaceR.ReplaceAllString(t, " "))
	case bool:
		return strconv.FormatBool(t)
	case float64:
		// encoding/json decodes every number as float64; render integers as
		// integers so `filesChanged: 4` does not arrive as `4.0`.
		if t == float64(int64(t)) {
			return strconv.FormatInt(int64(t), 10)
		}
		return strconv.FormatFloat(t, 'f', -1, 64)
	case nil:
		return "null"
	default:
		b, err := json.Marshal(t)
		if err != nil {
			return ""
		}
		return string(b)
	}
}

// cutFact applies the announced-ellipsis cap. TWIN: cut.
func cutFact(text string, uncapped bool) string {
	if uncapped || len(text) <= factValueMax {
		return text
	}
	return fmt.Sprintf("%s… (%d chars)", text[:factValueMax], len(text))
}

// renderCommitFact shortens a real 40-hex sha the way every other tool here
// shows one, and leaves an already-abbreviated sha, a tag or a sentence exactly
// as the worker reported it. TWIN: renderCommit.
func renderCommitFact(text string) string {
	if fullShaRe.MatchString(text) {
		return text[:12]
	}
	return text
}

// renderFact renders ONE key/value pair, or "" when the value carries no fact.
// TWIN: renderFact.
func renderFact(key string, value any) string {
	uncapped := uncappedFactKeys[strings.ToLower(key)]
	if value == nil {
		return ""
	}
	if list, ok := value.([]any); ok {
		items := make([]string, 0, len(list))
		for _, it := range list {
			if s := scalarFactText(it); s != "" {
				items = append(items, s)
			}
		}
		if len(items) == 0 {
			return ""
		}
		shown := items
		if !uncapped && len(items) > factItemsShown {
			shown = items[:factItemsShown]
		}
		body := strings.Join(shown, ", ")
		// "+K more" rather than a bare truncation: the reader is told a number
		// is missing and how big it is, which is the difference between lossy
		// and dishonest.
		if hidden := len(items) - len(shown); hidden > 0 {
			body = fmt.Sprintf("%s, +%d more", body, hidden)
		}
		return fmt.Sprintf("%s: %s", key, cutFact(body, uncapped))
	}
	text := scalarFactText(value)
	if text == "" {
		return ""
	}
	if strings.EqualFold(key, "commit") {
		return fmt.Sprintf("%s: %s", key, renderCommitFact(text))
	}
	return fmt.Sprintf("%s: %s", key, cutFact(text, uncapped))
}

// renderResultFacts renders a worker's parsed result as ONE compact run of
// facts. `result` is treated as ARBITRARY JSON because it is: the schema is
// written per dispatch and {commit, filesChanged, checksRun, caveats,
// followUps} is only the common shape. Known keys lead, in factOrder; the rest
// follow sorted. Empty values carry no fact and are dropped — an EMPTY caveats
// list is a truthful "none", not a caveat. TWIN: renderResultFacts.
func renderResultFacts(result map[string]any) string {
	if len(result) == 0 {
		return ""
	}
	rest := make([]string, 0, len(result))
	for k := range result {
		known := false
		for _, o := range factOrder {
			if o == k {
				known = true
				break
			}
		}
		if !known {
			rest = append(rest, k)
		}
	}
	sort.Strings(rest)

	parts := make([]string, 0, len(result))
	for _, k := range factOrder {
		if v, ok := result[k]; ok {
			if s := renderFact(k, v); s != "" {
				parts = append(parts, s)
			}
		}
	}
	for _, k := range rest {
		if s := renderFact(k, result[k]); s != "" {
			parts = append(parts, s)
		}
	}
	return strings.Join(parts, "; ")
}

// errBriefSignificanceRequired is the first hard rule's refusal.
var errBriefSignificanceRequired = errors.New(
	"brief.append: `line` must carry your own one-sentence significance line — " +
		"what this result MEANS for the project — and nothing was written without it. " +
		"The result object supplies the mechanical facts (commit, files, checks, " +
		"caveats) and the host renders them for you; the judgement is the part only " +
		"you can write, so a result on its own can never become a brief line.")

// composeResultLine builds the final line: date, the manager's sentence, the
// mechanical facts, the session reference. TWIN: composeResultLine.
//
// `now` is injected so the test can pin the date. Local time, not UTC: a brief
// is a human's dated log, so "today" means the user's today.
func composeResultLine(significance, sessionID string, result map[string]any, now time.Time) (string, error) {
	// flattenBriefLine (brief.go), NOT a \s+ collapse — interior spaces survive,
	// because a \s+ collapse also eats the double space in the doctrine's own
	// dated format (`- YYYY-MM-DD  <what happened>`), so a manager backfilling a
	// dated line would have its date separator quietly re-spaced by the one tool
	// that exists to write those lines. TWIN: flattenSentence.
	sentence := flattenBriefLine(significance)
	if sentence == "" {
		return "", errBriefSignificanceRequired
	}
	ref := ""
	if strings.TrimSpace(sessionID) != "" {
		var err error
		if ref, err = normalizeSessionRef(sessionID); err != nil {
			return "", err
		}
	}
	facts := renderResultFacts(result)

	// A sentence the caller already dated keeps ITS date: re-prefixing would
	// produce `- 2026-08-26  2026-08-26 …`, and the caller's date may be
	// deliberate (backfilling yesterday's entry).
	dated := sentence
	if !leadDateRe.MatchString(sentence) {
		dated = fmt.Sprintf("%s  %s", now.Format("2006-01-02"), sentence)
	}

	var b strings.Builder
	b.WriteString(dated)
	if facts != "" {
		b.WriteString(" — ")
		b.WriteString(facts)
	}
	// Likewise, a sentence that already names this session is not given a second
	// copy of the same reference.
	if ref != "" && !strings.Contains(strings.ToLower(dated), "session:"+ref) {
		b.WriteString(" (session:")
		b.WriteString(ref)
		b.WriteString(")")
	}
	return b.String(), nil
}
