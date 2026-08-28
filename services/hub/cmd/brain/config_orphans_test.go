package main

import (
	"os"
	"reflect"
	"strings"
	"testing"

	yaml "gopkg.in/yaml.v3"
)

// Retiring ONE orphaned config key.
//
// The supervisor ROLE was deleted (series merged at a6ad647d) but existing
// config.yaml files still carry its block, and config loading deep-merges with
// no unknown-key pruning — so it round-trips forever. These pin the two things
// that make removing it safe to ship against a user's real file: the blast
// radius is exactly one top-level key, and everything else (comments, key
// order, formatting, trailing newline) survives byte for byte.
//
// Twin: apps/desktop/src/main/lib/orphanedConfigKeys.test.ts. Both writers of
// config.yaml must agree, or whichever one still carries the key writes it
// straight back on its next save.

// prune parses raw the way loadFromDisk does and runs the pruner over it.
func prune(t *testing.T, raw string) (removed []string, text []byte, parsed map[string]any) {
	t.Helper()
	if err := yaml.Unmarshal([]byte(raw), &parsed); err != nil {
		t.Fatalf("fixture did not parse: %v", err)
	}
	if parsed == nil {
		parsed = map[string]any{}
	}
	removed, text = pruneOrphanedConfigKeys(parsed, []byte(raw))
	return removed, text, parsed
}

func TestOrphanedConfigKeysIsOnlyTheSupervisorBlock(t *testing.T) {
	if !reflect.DeepEqual(orphanedConfigKeys, []string{"supervisor"}) {
		t.Fatalf("orphanedConfigKeys = %v, want exactly [supervisor] — nothing else is up "+
			"for deletion, and this list is a twin of the desktop's ORPHANED_CONFIG_KEYS",
			orphanedConfigKeys)
	}
}

func TestPruneRemovesTheSupervisorBlock(t *testing.T) {
	raw := "ui:\n  theme: nord\nsupervisor:\n  provider: claude\n"
	removed, text, parsed := prune(t, raw)

	if !reflect.DeepEqual(removed, []string{"supervisor"}) {
		t.Fatalf("removed = %v, want [supervisor]", removed)
	}
	if _, ok := parsed["supervisor"]; ok {
		t.Errorf("supervisor still in the in-memory config: %#v", parsed)
	}
	if got, want := string(text), "ui:\n  theme: nord\n"; got != want {
		t.Errorf("text = %q, want %q", got, want)
	}

	// Idempotent: a second pass over the result writes nothing.
	removed2, text2, _ := prune(t, string(text))
	if len(removed2) != 0 || text2 != nil {
		t.Errorf("second pass was not a no-op: removed=%v text=%q", removed2, text2)
	}
}

func TestPruneLeavesAConfigWithoutTheBlockAlone(t *testing.T) {
	// The common case — every new install. No error, and no write at all.
	for _, raw := range []string{
		"ui:\n  theme: nord\nclaude:\n  defaultModel: opus\n",
		"",
		"# nothing here yet\n",
	} {
		removed, text, _ := prune(t, raw)
		if len(removed) != 0 {
			t.Errorf("removed = %v for %q, want none", removed, raw)
		}
		if text != nil {
			t.Errorf("text = %q for %q, want nil (nothing to write)", text, raw)
		}
	}
}

func TestPrunePreservesCommentsOrderAndFormatting(t *testing.T) {
	// A hand-annotated file: comments above, between and inside blocks, a
	// deliberately non-default key order, quoting, and a blank-line rhythm.
	raw := strings.Join([]string{
		"# my workspacer config — hand edited, do not clobber",
		"ui:",
		"  theme: nord # the only one I can read",
		"  # tried 15, too small",
		"  fontSize: 16",
		"",
		"# left over from the fleet supervisor experiment",
		"supervisor:",
		"  provider: claude",
		"  # cheap worker for digests",
		"  summarizerModel: haiku",
		"",
		"  models:",
		"    coordinator: opus",
		"",
		"claude:",
		"  defaultModel: 'opus'",
		"",
		"# projects last on purpose",
		"projects:",
		"  /home/u/proj:",
		"    label: Proj",
		"",
	}, "\n")

	// The input with exactly the block's own lines cut: the header comment
	// stays, the comment ABOVE the block stays (it is not part of the block),
	// the trailing blank separator goes with it.
	want := strings.Join([]string{
		"# my workspacer config — hand edited, do not clobber",
		"ui:",
		"  theme: nord # the only one I can read",
		"  # tried 15, too small",
		"  fontSize: 16",
		"",
		"# left over from the fleet supervisor experiment",
		"claude:",
		"  defaultModel: 'opus'",
		"",
		"# projects last on purpose",
		"projects:",
		"  /home/u/proj:",
		"    label: Proj",
		"",
	}, "\n")

	removed, text, parsed := prune(t, raw)
	if !reflect.DeepEqual(removed, []string{"supervisor"}) {
		t.Fatalf("removed = %v, want [supervisor]", removed)
	}
	if string(text) != want {
		t.Errorf("comments/order/formatting not preserved.\n got:\n%s\nwant:\n%s", text, want)
	}

	// Every surviving line must be an untouched line of the original.
	original := map[string]bool{}
	for _, line := range strings.Split(raw, "\n") {
		original[line] = true
	}
	for _, line := range strings.Split(string(text), "\n") {
		if !original[line] {
			t.Errorf("line %q is not a verbatim line of the original file", line)
		}
	}

	// And it still means the pruned document.
	var reparsed map[string]any
	if err := yaml.Unmarshal(text, &reparsed); err != nil {
		t.Fatalf("result did not parse: %v", err)
	}
	if !reflect.DeepEqual(reparsed, parsed) {
		t.Errorf("result means something other than the pruned config:\n%#v\n%#v", reparsed, parsed)
	}
}

func TestPruneAnchorsOnColumnZero(t *testing.T) {
	nested := "agents:\n  supervisor:\n    provider: claude\nui:\n  theme: nord\n"
	if removed, text, _ := prune(t, nested); len(removed) != 0 || text != nil {
		t.Errorf("a NESTED supervisor key was touched: removed=%v text=%q", removed, text)
	}
	prefixed := "supervisorLoop:\n  enabled: true\n"
	if got := string(stripTopLevelBlock([]byte(prefixed), "supervisor")); got != prefixed {
		t.Errorf("a key that merely starts with the name was touched: %q", got)
	}
}

func TestPruneFileShapeEdgeCases(t *testing.T) {
	cases := []struct {
		name    string
		raw     string
		want    string // "" means: refuse to write (text == nil)
		removed bool
	}{
		{"block ends the file", "ui:\n  theme: nord\nsupervisor:\n  provider: claude\n", "ui:\n  theme: nord\n", true},
		{"no trailing newline", "ui:\n  theme: nord\nsupervisor:\n  provider: claude", "ui:\n  theme: nord", true},
		{"block is first", "supervisor:\n  provider: claude\nui:\n  theme: nord\n", "ui:\n  theme: nord\n", true},
		{"empty (null) block", "supervisor:\nui:\n  theme: nord\n", "ui:\n  theme: nord\n", true},
		{"flow style on one line", "supervisor: {provider: claude}\nui:\n  theme: nord\n", "ui:\n  theme: nord\n", true},
		{"CRLF endings", "ui:\r\n  theme: nord\r\nsupervisor:\r\n  provider: claude\r\n", "ui:\r\n  theme: nord\r\n", true},
		// In-memory is cleaned, but the FILE is left exactly as it was: the
		// column-0 scan cannot match a quoted key, and emptying the file
		// outright is not a tidy-up.
		{"quoted key is not matched textually", "\"supervisor\":\n  provider: claude\nui:\n  theme: nord\n", "", true},
		{"file would be emptied", "supervisor:\n  provider: claude\n", "", true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			removed, text, parsed := prune(t, tc.raw)
			if got := len(removed) > 0; got != tc.removed {
				t.Fatalf("removed = %v, want removed=%v", removed, tc.removed)
			}
			if tc.removed {
				if _, ok := parsed["supervisor"]; ok {
					t.Errorf("in-memory config still carries supervisor: %#v", parsed)
				}
			}
			if tc.want == "" {
				if text != nil {
					t.Errorf("text = %q, want nil (refuse to write)", text)
				}
				return
			}
			if string(text) != tc.want {
				t.Errorf("text = %q, want %q", text, tc.want)
			}
		})
	}
}

// TestLoadFromDiskDropsTheSupervisorBlockOnce is the end-to-end half: a real
// config.yaml on disk, read through the real configService, must come back
// without the block AND be rewritten with everything else intact — with no
// second write on the next load.
func TestLoadFromDiskDropsTheSupervisorBlockOnce(t *testing.T) {
	tempConfigHome(t)

	raw := "# hand written\nui:\n  theme: nord\n\nsupervisor:\n  provider: claude\n  fullAccess: true\n\nclaude:\n  defaultModel: opus\n"
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(configPath(), []byte(raw), 0o644); err != nil {
		t.Fatal(err)
	}

	cfg := newConfigService().get()
	if _, ok := cfg["supervisor"]; ok {
		t.Errorf("supervisor survived the load: %#v", cfg["supervisor"])
	}

	onDisk, err := os.ReadFile(configPath())
	if err != nil {
		t.Fatal(err)
	}
	want := "# hand written\nui:\n  theme: nord\n\nclaude:\n  defaultModel: opus\n"
	if string(onDisk) != want {
		t.Fatalf("config.yaml on disk:\n%q\nwant:\n%q", onDisk, want)
	}

	// A second load must be a pure read — the file is already clean.
	before, err := os.Stat(configPath())
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := newConfigService().get()["supervisor"]; ok {
		t.Error("supervisor came back on the second load")
	}
	after, err := os.Stat(configPath())
	if err != nil {
		t.Fatal(err)
	}
	if !before.ModTime().Equal(after.ModTime()) || before.Size() != after.Size() {
		t.Error("the second load rewrote a config that was already clean")
	}
}
