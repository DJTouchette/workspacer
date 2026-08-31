package bus

// THE CANONICAL SPAWN KEY SET — what makes sanitizeSpawnParams' exact-key
// matching SOUND rather than merely conventional.
//
// THE BUG THIS EXISTS TO CLOSE. sanitizeSpawnParams reads params as a
// map[string]json.RawMessage and strips, stamps and clamps EXACT lower-camel
// keys: `yoloGranted`, `profileId`, `mcpFacade`, `toolScope`, `capability`,
// `model`, `effort`. Every provider on the far side of that map decodes it with
// `encoding/json` into a tagged struct — and encoding/json matches field tags
// CASE-INSENSITIVELY. So `{"YoloGranted":true}` survives a sanitizer that
// deletes `yoloGranted`, and binds to spawnParams.YoloGranted anyway. One
// capital letter and the hub's single spawn-authority gate is a no-op: the
// full-access stamp, the tool-tier clamp and the capability ceiling are all
// walked around at once, on a fleet whose agents run with permissions bypassed.
//
// WHY THE FIX IS HERE AND NOT IN THE PROVIDER. The Go brain has carried its own
// guard (rejectCaseVariantKeys) since b4309a45, but it only fires when BOTH
// spellings are present — a single capitalized key sails through — and, more
// importantly, it is one provider's guard. The placement claim this whole design
// rests on is that `methodSanitizers` is the single dispatch table for call()
// AND federatedCall(), so a rule written once here covers every path: the
// desktop provider, the headless brain, a peer's hub across the federated hop,
// the MCP facade, a plugin, a phone. A guard in cmd/brain covers cmd/brain.
//
// THE RULE, and why it is REJECT rather than canonicalize. A key that case-folds
// to a spawn param the hub knows about, but is not spelled exactly that way, is
// REFUSED and the call fails with an error naming the key and its canonical
// spelling. Silently rewriting `YoloGranted` to `yoloGranted` would work, and it
// would also mean an authority gate that quietly repairs malformed authority
// assertions — the one shape where "be liberal in what you accept" is the wrong
// instinct. No legitimate caller in this repo has ever sent one of these: the
// desktop, the MCP facade, the TUI, /m and the web client all emit the canonical
// spelling, so a non-canonical one is either a bug worth a loud error or an
// attempt worth refusing.
//
// KEYS THAT FOLD TO NOTHING KNOWN PASS THROUGH. A field this list has never
// heard of cannot be aliasing a field the sanitizer decides about, so refusing
// it would buy no safety and would break the next provider-only param before the
// hub learned its name. What is NOT tolerated is two keys of ANY spelling that
// fold together: that is genuine ambiguity, and the guard and the decoder could
// read different values from it.
//
// KEEPING THE LIST HONEST. A provider field missing from this list reopens the
// hole for exactly that field, so the list is not allowed to drift: the brain's
// own spawnParams struct is reflected over in
// TestEverySpawnParamFieldIsInTheBusCanonicalKeySet (cmd/brain), which fails the
// day a provider grows a tag this file does not name.

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
)

// spawnParamKeys is every top-level `agents.spawn` param key any provider in
// this repo decodes, in the ONE spelling that is allowed to arrive.
//
// Sourced from the three spawn surfaces, and it is the UNION rather than any
// one of them: the hub does not know which provider will answer, and a key the
// desktop reads must be pinned even on a headless node.
//
//   - services/hub/cmd/brain/handlers.go   spawnParams (the headless provider)
//   - apps/desktop/src/main/services/hubCapabilities.ts  the desktop provider
//   - services/hub/cmd/mcp/main.go         spawnAgentIn (the facade's caller side)
var spawnParamKeys = []string{
	// ── identity / grant fields the router itself strips or stamps ──────────
	"profileId",
	"profileGranted",
	"yoloGranted",
	"escalationScrubbed",
	// ── the authority axis ──────────────────────────────────────────────────
	"mcpFacade",
	"toolScope",
	"pluginTools",
	"mcpItemIds",
	"permissionMode",
	"skipPermissions",
	"fleetFullAccess",
	"manager",
	// ── the capability axis (the routing ceiling clamps these) ──────────────
	"capability",
	"model",
	"effort",
	"provider",
	"transport",
	// ── audit correlation. Not authority — but a case variant of one of these
	//    would make the decision log describe a spawn that did not happen. ───
	"role",
	"decisionId",
	// ── ordinary spawn shape ────────────────────────────────────────────────
	"cwd",
	"cols",
	"rows",
	"label",
	"parentSessionId",
	"resumeSessionId",
	"message",
	"worktree",
	"resultSchema",
	"template",
	"templateParams",
}

// spawnParamCanonical maps a case-folded spawn key to its one allowed spelling.
var spawnParamCanonical = func() map[string]string {
	m := make(map[string]string, len(spawnParamKeys))
	for _, k := range spawnParamKeys {
		m[strings.ToLower(k)] = k
	}
	return m
}()

// SpawnParamKeys is the canonical spelling of every agents.spawn param key the
// hub knows about, sorted.
//
// Exported for ONE reason: the drift guard in cmd/brain reflects over the
// headless provider's spawnParams struct and asserts every json tag appears
// here. A provider field this list does not name is a field whose case variants
// reach that provider unchecked, and the point of the guard is that adding one
// is a test failure rather than a quiet reopening of the hole.
func SpawnParamKeys() []string {
	out := append([]string(nil), spawnParamKeys...)
	sort.Strings(out)
	return out
}

// rejectAliasedSpawnKeys refuses a params object whose top-level keys would make
// the sanitizer and the provider's decoder read different things.
//
// Two refusals, in this order:
//
//  1. TWO KEYS THAT FOLD TOGETHER — of any spelling, known or not. The guard
//     would read one and encoding/json would read whichever it saw last.
//  2. A KEY THAT FOLDS TO A KNOWN SPAWN PARAM BUT IS NOT SPELLED AS ONE. This is
//     the authority bypass: `YoloGranted`, `Capability`, `MCPFacade`, `ToolScope`.
//
// Keys sorted before judging so the error names the same key every run — an
// authority refusal that reports a different field per invocation is one nobody
// can write a test against.
func rejectAliasedSpawnKeys(m map[string]json.RawMessage) error {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)

	folded := make(map[string]string, len(keys))
	for _, k := range keys {
		lower := strings.ToLower(k)
		if prev, dup := folded[lower]; dup {
			return fmt.Errorf(
				"agents.spawn: params carry %q and %q, which differ only by case — "+
					"the hub's authority gate matches exact field names and the provider's JSON decoder does not, "+
					"so the two would read different values. Send one spelling",
				prev, k)
		}
		folded[lower] = k
	}

	for _, k := range keys {
		canon, known := spawnParamCanonical[strings.ToLower(k)]
		if !known || canon == k {
			continue
		}
		return fmt.Errorf(
			"agents.spawn: params name %q, which is not how this field is spelled — it is %q. "+
				"The hub's spawn-authority gate (the full-access stamp, the tool-tier clamp, the capability ceiling) "+
				"matches field names EXACTLY, while a provider's JSON decoder matches them case-insensitively, so a "+
				"variant spelling would bind on the provider having been invisible to the gate. Refused rather than "+
				"quietly rewritten: an authority assertion is not something to repair on the caller's behalf",
			k, canon)
	}
	return nil
}
