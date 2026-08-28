package main

import (
	"bytes"
	"reflect"
	"regexp"
	"strings"

	yaml "gopkg.in/yaml.v3"
)

// orphanedConfigKeys are top-level config.yaml keys belonging to a REMOVED
// feature, so nothing reads them any more — and which config loading would
// otherwise round-trip forever, since deepMerge copies every source key whether
// or not the defaults know it.
//
// There is deliberately no general unknown-key pruning: a key can be absent
// from the defaults because its feature loads late or lives in a plugin, and
// deleting those would be config loss. Retirement is spelled ONE KEY AT A TIME.
//
//   - "supervisor" — the fleet-supervisor ROLE, deleted in the supervisor-removal
//     series merged at a6ad647d. Nothing in any runtime reads the block.
//
// Twin of ORPHANED_CONFIG_KEYS in apps/desktop/src/main/lib/orphanedConfigKeys.ts.
// BOTH writers of config.yaml must carry the same list: whichever one still
// knows the key writes it straight back on its next save.
var orphanedConfigKeys = []string{"supervisor"}

// stripTopLevelBlock deletes a top-level `key:` block from raw YAML TEXT,
// leaving every other byte — comments, key order, quoting, indentation —
// untouched.
//
// Deliberately not a yaml.Marshal round trip: marshalling the parsed document
// drops every comment and re-orders keys, so tidying one dead key would eat a
// user's annotated config. The caller re-parses the result and refuses to write
// unless it means exactly the same thing minus the key.
//
// A block runs from its `key:` line to the next non-blank unindented line.
// Blank lines inside it, and the blank separator after it, go with it — but
// only when a real line follows, so a block at EOF leaves the file's trailing
// newline alone. Column 0 is the anchor: a same-named key nested under
// something else is never touched.
//
// Mirrors stripTopLevelBlock in the desktop's lib/orphanedConfigKeys.ts.
func stripTopLevelBlock(raw []byte, key string) []byte {
	head := regexp.MustCompile(`^` + regexp.QuoteMeta(key) + `:(\s|$)`)
	// Splitting on "\n" keeps a CRLF file's "\r" at the end of each line, so
	// re-joining is byte-identical for everything we did not remove.
	lines := strings.Split(string(raw), "\n")
	out := make([]string, 0, len(lines))
	for i := 0; i < len(lines); {
		if !head.MatchString(strings.TrimSuffix(lines[i], "\r")) {
			out = append(out, lines[i])
			i++
			continue
		}
		i++ // the `key:` line itself
		// Consume the body. Blank lines are held back until we know whether a
		// block line still follows: trailing blanks before the next key are the
		// block's own separator (drop them), trailing blanks at EOF are the
		// file's ending (keep them).
		var pending []string
		for i < len(lines) {
			line := strings.TrimSuffix(lines[i], "\r")
			if strings.TrimSpace(line) == "" {
				pending = append(pending, lines[i])
				i++
				continue
			}
			if strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t") {
				pending = nil
				i++
				continue
			}
			break
		}
		if i >= len(lines) {
			out = append(out, pending...) // EOF: keep the file's tail
		}
	}
	return []byte(strings.Join(out, "\n"))
}

// pruneOrphanedConfigKeys deletes every orphanedConfigKeys entry from a
// freshly-parsed config and, when it can be done without changing anything
// else, returns the file text to persist.
//
// parsed is MUTATED whether or not a write is possible, so this process never
// carries a dead key even if the file on disk keeps it for now. A nil text with
// a non-empty removed means "in memory only — leave the file exactly as it is".
//
// Idempotent: a config with no orphaned key returns (nil, nil) and the caller
// writes nothing, which is the common case.
//
// Mirrors pruneOrphanedConfigKeys in the desktop's lib/orphanedConfigKeys.ts.
func pruneOrphanedConfigKeys(parsed map[string]any, raw []byte) (removed []string, text []byte) {
	for _, key := range orphanedConfigKeys {
		if _, ok := parsed[key]; ok {
			removed = append(removed, key)
		}
	}
	if len(removed) == 0 {
		return nil, nil
	}

	text = raw
	for _, key := range removed {
		text = stripTopLevelBlock(text, key)
	}
	for _, key := range removed {
		delete(parsed, key)
	}

	if bytes.Equal(text, raw) {
		return removed, nil // nothing matched textually
	}

	// The safety net. Text surgery on YAML may only change the ONE thing it
	// claims to: re-read what we produced and require that it means exactly the
	// pruned document. Anything else (a quoted key we did not match, an anchor,
	// a multi-doc file, a block we mis-bounded) leaves the file alone rather
	// than writing a config we cannot prove is equivalent.
	var reparsed map[string]any
	if err := yaml.Unmarshal(text, &reparsed); err != nil {
		return removed, nil
	}
	// Note a file that was ONLY the orphaned block unmarshals to nil, not to an
	// empty map, so it fails this check and is left alone — same as the TS twin,
	// where yaml.load("") is undefined against a {} parse. Emptying config.yaml
	// entirely is not a tidy-up, and loadFromDisk's persistBlocked branch owns
	// that shape anyway.
	if !reflect.DeepEqual(reparsed, parsed) {
		return removed, nil
	}
	return removed, text
}
