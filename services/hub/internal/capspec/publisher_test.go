package capspec

import (
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"
)

// THE FORCING FUNCTION FOR THE PUBLISH DIRECTION.
//
// EventTopic.Method answers "who may HEAR this". Publisher answers "who may SAY
// it", and until the provider tier existed the second question had exactly one
// answer everywhere — the host — so it did not need a field. It does now: a
// provider-tier connection (a remote node's `brain --hub`) is not trusted, and
// mayPublish admits it to a classified topic if and only if it registered the
// capability this field names. A brain topic whose row leaves Publisher empty
// is therefore not "unset", it is MUTE: the feed silently dies on a headless
// node and the failure looks like a broken brain rather than a missing row.
//
// So the rule is scanned rather than listed, the same way
// TestEveryPublishedTopicIsClassified scans the publish sites: every topic
// cmd/brain publishes must name a Publisher. A new brain feed cannot ship
// without someone deciding which capability answers for it.

// brainPublishRe matches cmd/brain's thin `publish("topic", …)` wrapper — the
// one publish shape that file tree uses. Both forms: a whole literal, and a
// literal PREFIX (`"pty.bytes."+f.id`), which is a family.
var brainPublishRe = regexp.MustCompile(`\bpublish\(\s*"([\w.]+)"(\s*\+)?`)

// brainPublishFloor ratchets the scan. A regex that stops matching returns zero
// topics, the loop below runs zero times, and the guard reports PASS — the
// failure mode every scan-based test in this repo has had at least once.
const brainPublishFloor = 4

func TestEveryTopicTheHeadlessProviderPublishesNamesAPublisher(t *testing.T) {
	root := mustRepoRoot(t)
	files := walkFiles(t, filepath.Join(root, "services", "hub", "cmd", "brain"), func(name string) bool {
		return strings.HasSuffix(name, ".go") && !strings.HasSuffix(name, "_test.go")
	})

	found := map[string]string{} // topic (or "prefix*") -> file
	for _, f := range files {
		for _, m := range brainPublishRe.FindAllStringSubmatch(f.body, -1) {
			topic := m[1]
			if strings.TrimSpace(m[2]) == "+" {
				topic += "*"
			}
			found[topic] = f.rel
		}
	}
	if len(found) < brainPublishFloor {
		t.Fatalf("scanned only %d topics out of cmd/brain (floor %d) — the brain's publish shape changed and this guard is guarding nothing. Found: %v",
			len(found), brainPublishFloor, sortedTopics(found))
	}

	for _, topic := range sortedTopics(found) {
		probe := strings.TrimSuffix(topic, "*")
		if strings.HasSuffix(topic, "*") {
			probe += "probe"
		}
		spec, ok := EventTopicSpec(probe)
		if !ok {
			// TestEveryPublishedTopicIsClassified already fails on this; do not
			// report the same defect twice under a different name.
			continue
		}
		if spec.Publisher == "" {
			t.Errorf("%s publishes %q and its registry row names no Publisher. A headless capability provider is NOT trusted, so mayPublish refuses it every classified topic whose Publisher is empty — this feed is dead on a remote node. Name the capability whose provider emits it, or record why the topic may only ever come from the host.",
				found[topic], topic)
		}
	}
}

// A Publisher naming a method capspec says nothing about is a grant nobody can
// reason about — the same rule TestEventTopicRegistryIsWellFormed applies to
// Method, applied to the field that now hands out an authority.
func TestEveryPublisherNamesAClassifiedCapability(t *testing.T) {
	named := 0
	for _, r := range EventTopics() {
		if r.Publisher == "" {
			continue
		}
		named++
		if strings.Contains(r.Publisher, "*") {
			t.Errorf("%q names Publisher %q — a PATTERN. Publisher is fed to mayProvide as a concrete method name; a wildcard here would grant the publish side of every capability matching it.", r.Pattern, r.Publisher)
		}
		if MissingClassification(r.Publisher) {
			t.Errorf("%q names Publisher %q, which capspec says nothing about — a publish grant pointing at an unclassified method cannot be reasoned about", r.Pattern, r.Publisher)
		}
	}
	if named < brainPublishFloor {
		t.Fatalf("only %d rows name a Publisher (want >= %d: the topics the headless provider emits). The field emptied out, and every headless feed it governs is now refused.", named, brainPublishFloor)
	}
}

// The negative half, and the one that matters most: a topic whose forged form
// is an attack must NOT acquire a publisher by drift.
//
// layout.changed carries the four spawn-escalation fields layout.set scrubs
// from a non-trusted writer, plus a publisher-chosen version that wins every
// later comparison; plugin.settings.changed carries plugin endpoints, org/repo
// names and absolute paths. Neither is any provider's output, so neither may
// ever be sayable by a credential that merely registered a capability.
func TestForgeryTargetsHaveNoPublisher(t *testing.T) {
	for _, topic := range []string{
		"layout.changed",
		"plugin.settings.changed",
		"plugin.log",
		"agent.state_changed", // published by the desktop's TS side, not by any brain
		"node.state_changed",
		"plugin.install.progress",
	} {
		spec, ok := EventTopicSpec(topic)
		if !ok {
			t.Fatalf("%q is not in the registry at all — this test is checking nothing", topic)
		}
		if spec.Publisher != "" {
			t.Errorf("%q now names Publisher %q, so any connection that registers %q may publish it. If that is genuinely intended, the reason belongs in the row and this list; until then it is a forgery channel opened by a field edit.",
				topic, spec.Publisher, spec.Publisher)
		}
	}
}

func sortedTopics(m map[string]string) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}
