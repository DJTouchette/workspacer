package routing

// THE CONTEXT-WINDOW SUFFIX, AND WHY THE MATRIX HAS TO LOOK PAST IT.
//
// `opus[1m]` is not a different model from `opus`. It is the same model with a
// different CONTEXT WINDOW: 1M rather than the standard 200K. Claude Code takes
// the marker on the spawn command line and strips it back out of the id it
// writes into the transcript, which is why contracts/model-context-windows.json
// treats the two spellings (`[1m]` and the `-1m` id suffix) as a statement about
// the window and never about the family.
//
// The routing matrix spells the model `opus`, and the desktop's shipped
// `claude.defaultModel` is `opus[1m]`. So a spawn that simply omitted `model`
// arrived at the ceiling carrying a string the matrix had no entry for, the
// named-model arm found no reading for it, and an unjudgeable model is not
// judged at all. The strongest model on the Claude path was the one the ceiling
// could not see, reached by leaving a field out.
//
// THE FIX IS AT THE LOOKUP, NOT IN THE FILE. Adding `opus[1m]` rows beside every
// `opus` row would make routing.yaml drift against itself: one model deserves
// one entry, and the second copy is the one someone forgets to edit. So the
// suffix is taken off for the COMPARISON and nowhere else.
//
// AND ONLY FOR THE COMPARISON. The suffix has to survive to the actual spawn:
// strip it from the model the provider is handed and every dispatch silently
// loses its 1M window, which nobody notices until an agent runs out of room.
// Nothing here rewrites a request. capabilityOfModel and providerOfModel call
// it to decide what a caller's string MEANS; the string itself travels on
// untouched, and the one path that replaces a model (routeSafely) writes the
// matrix entry verbatim.

import "strings"

// windowSuffixes are the two spellings of the 1M context-window request, as
// contracts/model-context-windows.json's `windows` block names them. They are
// matched as SUFFIXES here rather than anywhere in the string, which is the
// narrower reading: `opus[1m]`, `claude-opus-5[1m]` and `claude-sonnet-5-1m` are
// all a model plus a window, while a `-1m` in the middle of an id is part of the
// id.
var windowSuffixes = []string{"[1m]", "-1m"}

// splitModelWindowSuffix separates a model id from a trailing context-window
// request. The suffix comes back as it was written, so a caller can say what was
// dropped; it is "" when the id carries none.
//
// A model id that is NOTHING BUT a suffix keeps it: "-1m" alone names no model,
// and returning an empty base would turn it into "the matrix has no reading",
// which is a different and much more permissive answer.
func splitModelWindowSuffix(model string) (base, suffix string) {
	m := strings.TrimSpace(model)
	for _, s := range windowSuffixes {
		if len(m) > len(s) && strings.EqualFold(m[len(m)-len(s):], s) {
			return m[:len(m)-len(s)], m[len(m)-len(s):]
		}
	}
	return m, ""
}

// matchableModel is the form a model id is COMPARED in: lowercased, trimmed, and
// with any context-window suffix taken off. Both sides of every matrix
// comparison go through it, so a profile entry written `model: opus[1m]` matches
// a spawn naming `opus` and vice versa. One model, one entry, whatever window
// either side asked for.
func matchableModel(model string) string {
	base, _ := splitModelWindowSuffix(model)
	return strings.ToLower(base)
}
