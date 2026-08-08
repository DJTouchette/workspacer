package capspec

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// THE COMPLETENESS GUARD FOR THE EVENT PLANE.
//
// The call plane has three of these — TestDesktopCapabilitiesAllClassified,
// TestBrainMethodsAllClassified, TestHubNativeCapabilitiesAllClassified — and
// each works the same way: enumerate what the CODE registers, then demand
// capspec say something about every name. The event plane had nothing of the
// kind. TestEventTopicGuardsNameClassifiedMethods walks the guard TABLE and
// checks the rows already in it, which is a table validating itself; a topic
// nobody wrote down was invisible to it, and 23 of the 25 published topics were
// exactly that.
//
// So this scans the PUBLISH SITES, in both languages, and fails on a topic the
// registry does not classify. It is the forcing function: a new topic cannot
// ship until someone decides whether it is guarded by a capability, host-only,
// or open by decision — and writes the reason down.
//
// The scanner is deliberately intolerant of a publish site it cannot read. A
// site whose topic argument is neither a literal, a literal prefix, nor a
// resolvable constant fails and must be added to passthroughSites with a reason,
// because "the scanner did not understand this line" and "there is nothing here
// to classify" have to be different outcomes. That distinction is the one every
// scan-based guard in this repo has had to learn: a regex that silently matches
// nothing reports PASS.

// goTopicFloor / tsTopicFloor are RATCHETS on what the scan finds, not on the
// registry. A scan that stops matching returns zero topics, every loop below
// runs zero times, and the guard reports ok — the failure mode this whole family
// of tests keeps re-learning. Raise them when a topic is added.
const (
	goTopicFloor = 13
	tsTopicFloor = 7
	// publishSiteFloor is the number of publish CALL SITES the scan must
	// account for across both languages. It catches the other direction: a
	// scanner that still resolves the topics it knows while a new call site
	// slips past the regex entirely.
	publishSiteFloor = 20
)

// goPublishRe finds the topic argument of a Go publish site.
//
// Three forms, because the code has three: event.New("x", …) constructs the
// envelope; brain's thin bus.publish("x", payload) wrapper; and a bare
// event.Envelope{Type: "x"} literal, which is how pty.desync is synthesized —
// a form the first two regexes would have missed entirely, and pty.desync is the
// leak that started this file.
var (
	goEventNewRe = regexp.MustCompile(`event\.New\(\s*(?:"([\w.]+)"(\s*\+)?|([A-Za-z_][\w.]*))`)
	goPublishRe  = regexp.MustCompile(`\bpublish\(\s*(?:"([\w.]+)"(\s*\+)?|([A-Za-z_][\w.]*))`)
	goEnvelopeRe = regexp.MustCompile(`event\.Envelope\{[\s\S]{0,200}?Type:\s*"([\w.]+)"`)
	// Any call that puts something on the bus, for the accounting pass.
	goPublishSiteRe = regexp.MustCompile(`\.[Pp]ublish\(|\bpublish\(`)
	// const ChangedTopic = "layout.changed" — resolvable indirection.
	goTopicConstRe = regexp.MustCompile(`(?m)^\s*(?:const\s+)?([A-Za-z_]\w*)\s*=\s*"([\w.]+)"\s*$`)
)

// tsPublishRe finds publishToHub's topic. Both the quoted literal and the
// template form (`pty.bytes.${id}`, `workflow.${run.status}`), which is a
// PREFIX site: the suffix is chosen at runtime.
var (
	tsPublishTypeRe = regexp.MustCompile(`publishToHub\(\{[\s\S]{0,120}?type:\s*(?:'([\w.]+)'|"([\w.]+)"|` + "`" + `([\w.]*)\$\{)`)
	tsPublishSiteRe = regexp.MustCompile(`publishToHub\(`)
)

// passthroughSites are publish call sites that carry an envelope built
// elsewhere, so there is no topic here to classify. Each names WHY, and each is
// verified to still exist — an entry for a line nobody publishes any more is how
// an allowlist rots into permission.
//
// The rule for adding one: the topic must be classified at the site that
// CONSTRUCTS the envelope, or be a client-supplied type that mayPublish already
// governs. Anything else is a hole with a comment on it.
var passthroughSites = map[string]string{
	"internal/bus/bus.go:s.broker.Publish(*f.Event)":            "a client-supplied envelope: its type is whatever the client sent. mayPublish is the guard — a non-trusted connection may not publish a CLASSIFIED topic at all (host state is not forgeable), and an unclassified one is bounded by the manifest emits list.",
	"internal/claudemon/bridge.go:b.pub.Publish(ev)":            "re-publishes the envelope mapEvent built in the same file, whose literal (agent.state_changed) is scanned at that site.",
	"internal/supervisor/supervisor.go:Publish(event.Envelope)": "the Publisher interface method declaration, not a call.",
	"internal/plugin/manager.go:Publish(event.Envelope)":        "the Publisher interface method declaration, not a call.",
	"internal/claudemon/bridge.go:Publish(event.Envelope)":      "the Publisher interface method declaration, not a call.",
	"internal/broker/broker.go:func (b *Broker) Publish(":       "the broker's own fan-out entry point.",
	"cmd/brain/bus.go:func (b *busClient) publish(":             "brain's publish helper. Every caller passes a literal, and those are scanned at their call sites.",
	"cmd/brain/events.go:publish(u.SessionID, u.StatusLine)":    "invokes the injected callback whose topic literal (agent.statusline) is at cmd/brain/main.go, scanned there. The parameter is a func, not a topic.",
	"cmd/brain/library.go:workspacer.publish(type, data)":       "a line of DOCUMENTATION inside a generated skill file describing the plugin API to an agent — text, not a call.",
	"main/ipc.ts:publishToHub(ev);":                             "IPC.HUB_PUBLISH — the renderer publishing over the desktop's own TRUSTED host connection. Same trust domain and same process tree as main; the hub cannot distinguish them, and the topic is whatever the renderer chose.",
	"main/services/hubClient.ts:export function publishToHub(":  "the publish helper itself, not a call.",
}

// TestEveryPublishedTopicIsClassified is the forcing function.
func TestEveryPublishedTopicIsClassified(t *testing.T) {
	root := mustRepoRoot(t)

	goTopics, goPrefixes, goSites := scanGo(t, root)
	tsTopics, tsPrefixes, tsSites := scanTS(t, root)

	if len(goTopics)+len(goPrefixes) < goTopicFloor {
		t.Fatalf("scanned only %d Go topics (floor %d) — the publish syntax changed and this guard is guarding nothing. Found: %v %v",
			len(goTopics)+len(goPrefixes), goTopicFloor, sortedKeys(goTopics), sortedKeys(goPrefixes))
	}
	if len(tsTopics)+len(tsPrefixes) < tsTopicFloor {
		t.Fatalf("scanned only %d TS topics (floor %d) — publishToHub's call shape changed. Found: %v %v",
			len(tsTopics)+len(tsPrefixes), tsTopicFloor, sortedKeys(tsTopics), sortedKeys(tsPrefixes))
	}
	if goSites+tsSites < publishSiteFloor {
		t.Fatalf("accounted for only %d publish call sites (floor %d) — sites are being skipped before they are ever classified", goSites+tsSites, publishSiteFloor)
	}
	// Exact topics: the registry must name them.
	for _, topic := range append(sortedKeys(goTopics), sortedKeys(tsTopics)...) {
		if _, ok := EventTopicSpec(topic); !ok {
			t.Errorf("%s publishes %q and the event-topic registry says nothing about it. Every scoped user token is now REFUSED an unclassified topic, so this is a broken feed as well as an unmade decision: add a row to eventtopics.go saying it is guarded by a capability, host-only, or open by decision — with the reason.",
				firstOf(goTopics, tsTopics, topic), topic)
		}
	}

	// Prefix sites (`pty.bytes.` + id, `workflow.${status}`): at least one row
	// must cover the family. A suffix nobody listed is refused at runtime, which
	// is fail-closed and therefore a bug rather than a leak — but a family with
	// NO row is a family nobody considered.
	for _, prefix := range append(sortedKeys(goPrefixes), sortedKeys(tsPrefixes)...) {
		if !familyCovered(prefix) {
			t.Errorf("%s publishes topics beginning %q (the suffix is chosen at runtime) and no registry row covers that family. Classify the family with a %q row, or enumerate the concrete topics.",
				firstOf(goPrefixes, tsPrefixes, prefix), prefix, prefix+"*")
		}
	}

	// And the other direction: a registry row that no publish site can produce is
	// a guard on nothing, and would hide the rename that silenced a real one.
	produced := map[string]bool{}
	for k := range goTopics {
		produced[k] = true
	}
	for k := range tsTopics {
		produced[k] = true
	}
	prefixes := map[string]bool{}
	for k := range goPrefixes {
		prefixes[k] = true
	}
	for k := range tsPrefixes {
		prefixes[k] = true
	}
	for _, row := range EventTopics() {
		if produced[row.Pattern] {
			continue
		}
		covered := false
		for p := range prefixes {
			if strings.HasPrefix(row.Pattern, p) || strings.HasPrefix(p, strings.TrimSuffix(row.Pattern, "*")) {
				covered = true
				break
			}
		}
		if !covered {
			t.Errorf("the registry classifies %q and nothing in this repo publishes it. Either the publisher was renamed (and its new name is now unclassified) or the row is dead — a registry row that cannot fire is indistinguishable from one that works.", row.Pattern)
		}
	}
}

// TestEventTopicRegistryIsWellFormed holds each row to the shape that makes it
// mean something. A host-only or open-by-decision row IS its reason; without one
// it is a silent re-opening of the default this file exists to close.
func TestEventTopicRegistryIsWellFormed(t *testing.T) {
	rows := EventTopics()
	if len(rows) < goTopicFloor {
		t.Fatalf("the registry holds %d rows — it shrank, and every unclassified topic is now refused to scoped tokens", len(rows))
	}
	seen := map[string]bool{}
	byDisposition := map[EventTopicDisposition]int{}
	for _, r := range rows {
		if seen[r.Pattern] {
			t.Errorf("two rows classify %q — a failure would name the wrong one", r.Pattern)
		}
		seen[r.Pattern] = true
		byDisposition[r.Disposition]++
		if len(strings.TrimSpace(r.Reason)) < 60 {
			t.Errorf("%q has no real reason (%q). The reason is the whole content of a row: for a guarded topic, why that capability owns this payload; for the other two, what a receiver learns and why that is acceptable.", r.Pattern, r.Reason)
		}
		switch r.Disposition {
		case TopicGuardedBy:
			if r.Method == "" {
				t.Errorf("%q is guarded by nothing", r.Pattern)
				continue
			}
			if MissingClassification(r.Method) {
				t.Errorf("%q is guarded by %q, which capspec says nothing about — a guard pointing at an unclassified method cannot be reasoned about", r.Pattern, r.Method)
			}
		case TopicHostOnly, TopicOpenByDecision:
			if r.Method != "" {
				t.Errorf("%q is %q and also names method %q — the disposition decides, and two answers is none", r.Pattern, r.Disposition, r.Method)
			}
		default:
			t.Errorf("%q has disposition %q, which is not one of the three", r.Pattern, r.Disposition)
		}
	}
	// All three dispositions must be in use. TopicHostOnly is the one added this
	// round, for the topics NO capability mirrors; if it ever empties, the rule
	// has quietly reverted to "mirrors a capability", under which plugin.log is
	// unclassifiable and therefore open.
	for _, d := range []EventTopicDisposition{TopicGuardedBy, TopicHostOnly, TopicOpenByDecision} {
		if byDisposition[d] == 0 {
			t.Errorf("no row uses disposition %q — the registry has collapsed to a rule that cannot express every topic", d)
		}
	}
}

// TestEventTopicLookupMatchesItsOwnPatterns pins the matcher against the
// patterns it is given, including the exact-beats-prefix rule.
func TestEventTopicLookupMatchesItsOwnPatterns(t *testing.T) {
	for _, r := range EventTopics() {
		probe := r.Pattern
		if strings.HasSuffix(probe, "*") {
			probe = strings.TrimSuffix(probe, "*") + "anything"
		}
		got, ok := EventTopicSpec(probe)
		if !ok {
			t.Errorf("EventTopicSpec(%q) found nothing, but %q is in the registry", probe, r.Pattern)
			continue
		}
		if got.Pattern != r.Pattern {
			t.Errorf("EventTopicSpec(%q) resolved to %q, want %q", probe, got.Pattern, r.Pattern)
		}
	}
	// pty.exit is an exact row living under no wildcard; sidecar.running is a
	// wildcard hit. Both must resolve, and an invented topic must not.
	if _, ok := EventTopicSpec("sidecar.running"); !ok {
		t.Error("sidecar.running is not covered by the sidecar.* row")
	}
	if _, ok := EventTopicSpec("totally.invented.topic"); ok {
		t.Error("an invented topic resolved — the registry is matching everything, which is the open default in a new costume")
	}
	// The compatibility view must not report a host-only topic as unguarded in a
	// way a caller could read as "open".
	if _, guarded := EventTopicCapability("plugin.log"); guarded {
		t.Error("EventTopicCapability claims plugin.log is capability-guarded; it is host-only")
	}
	if !EventTopicHostOnly("plugin.log") {
		t.Error("plugin.log is not host-only — the topic that motivated the disposition")
	}
	if !EventTopicIsHostOwned("agent.snapshot") {
		t.Error("agent.snapshot is not host-owned, so a plugin could forge the payload push turns into a lock-screen notification")
	}
	if EventTopicIsHostOwned("example.clock.tick") {
		t.Error("a plugin-defined topic is being treated as host state — plugins could no longer emit their own events")
	}
}

// ---- scanning ---------------------------------------------------------

func mustRepoRoot(t *testing.T) string {
	t.Helper()
	root, err := sweepguard.Root()
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout: %v", err)
		}
		t.Fatalf("%v", err)
	}
	return root
}

func scanGo(t *testing.T, root string) (topics, prefixes map[string]string, sites int) {
	t.Helper()
	topics, prefixes = map[string]string{}, map[string]string{}
	consts := map[string]string{}
	files := walkFiles(t, filepath.Join(root, "services", "hub"), func(name string) bool {
		return strings.HasSuffix(name, ".go") && !strings.HasSuffix(name, "_test.go")
	})
	// Two passes: constants first, so an indirect topic argument resolves.
	for _, f := range files {
		for _, m := range goTopicConstRe.FindAllStringSubmatch(f.body, -1) {
			consts[m[1]] = m[2]
		}
	}
	for _, f := range files {
		record := func(m []string, rel string) {
			switch {
			case m[1] != "" && strings.TrimSpace(m[2]) == "+":
				prefixes[m[1]] = rel
			case m[1] != "":
				topics[m[1]] = rel
			case m[3] != "":
				name := m[3]
				if i := strings.LastIndex(name, "."); i >= 0 {
					name = name[i+1:]
				}
				if v, ok := consts[name]; ok {
					topics[v] = rel
				}
				// An unresolvable identifier is not silently dropped: the
				// accounting pass below is what fails on it.
			}
		}
		for _, m := range goEventNewRe.FindAllStringSubmatch(f.body, -1) {
			record(m, f.rel)
		}
		for _, m := range goPublishRe.FindAllStringSubmatch(f.body, -1) {
			record(m, f.rel)
		}
		for _, m := range goEnvelopeRe.FindAllStringSubmatch(f.body, -1) {
			topics[m[1]] = f.rel
		}
		sites += accountSites(t, f, goPublishSiteRe, func(line string) bool {
			return lineHasTopic(line, goEventNewRe, goPublishRe, consts)
		})
	}
	return topics, prefixes, sites
}

func scanTS(t *testing.T, root string) (topics, prefixes map[string]string, sites int) {
	t.Helper()
	topics, prefixes = map[string]string{}, map[string]string{}
	files := walkFiles(t, filepath.Join(root, "apps", "desktop", "src", "main"), func(name string) bool {
		return strings.HasSuffix(name, ".ts") && !strings.HasSuffix(name, ".test.ts")
	})
	for _, f := range files {
		for _, m := range tsPublishTypeRe.FindAllStringSubmatch(f.body, -1) {
			switch {
			case m[1] != "":
				topics[m[1]] = f.rel
			case m[2] != "":
				topics[m[2]] = f.rel
			case m[3] != "":
				prefixes[m[3]] = f.rel
			}
		}
		sites += accountSites(t, f, tsPublishSiteRe, func(line string) bool {
			return tsPublishTypeRe.MatchString(line)
		})
	}
	return topics, prefixes, sites
}

type srcFile struct {
	rel   string
	body  string
	lines []string
}

func walkFiles(t *testing.T, dir string, keep func(string) bool) []srcFile {
	t.Helper()
	var out []srcFile
	err := filepath.WalkDir(dir, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			switch d.Name() {
			case "node_modules", "vendor", ".git", "dist", "testdata":
				return fs.SkipDir
			}
			return nil
		}
		if !keep(d.Name()) {
			return nil
		}
		body, err := os.ReadFile(p)
		if err != nil {
			return err
		}
		rel := p
		if r, e := filepath.Rel(dir, p); e == nil {
			rel = filepath.ToSlash(filepath.Join(filepath.Base(dir), r))
		}
		out = append(out, srcFile{rel: rel, body: string(body), lines: strings.Split(string(body), "\n")})
		return nil
	})
	if err != nil {
		t.Fatalf("walking %s: %v", dir, err)
	}
	if len(out) == 0 {
		t.Fatalf("no source files under %s — the scan has nothing to read and would report PASS", dir)
	}
	return out
}

// accountSites is the half that makes the scan honest: every line that puts
// something on the bus must either carry a topic this scanner resolved, or be a
// recorded passthrough. A line that is neither fails, naming itself.
func accountSites(t *testing.T, f srcFile, siteRe *regexp.Regexp, hasTopic func(string) bool) int {
	t.Helper()
	n := 0
	for i, line := range f.lines {
		if !siteRe.MatchString(line) {
			continue
		}
		trimmed := strings.TrimSpace(line)
		// The declaration of a publish helper is not a publish.
		if strings.HasPrefix(trimmed, "//") || strings.HasPrefix(trimmed, "*") {
			continue
		}
		n++
		if hasTopic(line) {
			continue
		}
		// A multi-line call: event.New( on one line, the topic on the next.
		joined := line
		for j := i + 1; j < len(f.lines) && j <= i+3; j++ {
			joined += "\n" + f.lines[j]
		}
		if hasTopic(joined) {
			continue
		}
		if reason := passthroughReason(f.rel, trimmed); reason != "" {
			continue
		}
		t.Errorf("%s:%d publishes something this scanner cannot resolve to a topic:\n\t%s\nEither spell the topic as a literal, or add it to passthroughSites with the reason there is nothing to classify. A publish site the guard cannot read is a topic nobody classified.", f.rel, i+1, trimmed)
	}
	return n
}

func passthroughReason(rel, line string) string {
	for key, reason := range passthroughSites {
		i := strings.LastIndex(key, ":")
		if i < 0 {
			continue
		}
		file, frag := key[:i], key[i+1:]
		if strings.HasSuffix(rel, file) && strings.Contains(line, frag) {
			return reason
		}
	}
	return ""
}

func lineHasTopic(line string, eventNew, publish *regexp.Regexp, consts map[string]string) bool {
	for _, re := range []*regexp.Regexp{eventNew, publish} {
		for _, m := range re.FindAllStringSubmatch(line, -1) {
			if m[1] != "" {
				return true
			}
			name := m[3]
			if i := strings.LastIndex(name, "."); i >= 0 {
				name = name[i+1:]
			}
			if _, ok := consts[name]; ok {
				return true
			}
		}
	}
	return goEnvelopeRe.MatchString(line)
}

func familyCovered(prefix string) bool {
	for _, r := range EventTopics() {
		if strings.HasPrefix(r.Pattern, prefix) {
			return true
		}
		if strings.HasSuffix(r.Pattern, "*") && strings.HasPrefix(prefix, strings.TrimSuffix(r.Pattern, "*")) {
			return true
		}
	}
	return false
}

func sortedKeys(m map[string]string) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func firstOf(a, b map[string]string, key string) string {
	if v, ok := a[key]; ok {
		return v
	}
	if v, ok := b[key]; ok {
		return v
	}
	return fmt.Sprintf("<unknown site for %q>", key)
}

// The consent dialog is the renderer's copy of the guarded rows, and it was the
// stated justification for plugins being exempt from this registry at all:
// "a plugin's event reach is its manifest's `consumes`, declared at install and
// shown in the consent dialog, which is a real answer to the same question."
// It marked a consume line sensitive only for the exact pattern "*", so
// `consumes: ["pty.bytes.*", "fs.changed"]` rendered at severity=normal while
// the SAME reach spelled as capabilities rendered sensitive on both lines.
//
// The exemption is gone (mayConsume's plugin arm consults this registry now),
// but the dialog still has to describe what is being asked for — and a
// hand-copied table in another language is the drift shape this whole effort
// keeps closing. Every TopicGuardedBy row must appear there, with the same
// method.
func TestConsentDialogFlagsEveryGuardedTopic(t *testing.T) {
	body := string(mustReadRepoFile(t, "apps", "desktop", "src", "renderer", "src", "lib", "pluginPermissions.ts"))
	if !strings.Contains(body, "GUARDED_TOPICS") {
		t.Fatal("pluginPermissions.ts no longer has a GUARDED_TOPICS table — the consent dialog has stopped saying which consume lines carry a capability's output, and this guard is reading nothing")
	}
	checked := 0
	for _, row := range EventTopics() {
		if row.Disposition != TopicGuardedBy {
			continue
		}
		checked++
		want := "'" + row.Pattern + "': '" + row.Method + "'"
		if !strings.Contains(body, want) {
			t.Errorf("the install-consent dialog does not map %s — a manifest asking for this topic renders as an ordinary line while the capability behind it renders sensitive. Add %s to GUARDED_TOPICS in pluginPermissions.ts.", want, want)
		}
	}
	if checked < 4 {
		t.Fatalf("only %d guarded rows to check — the registry lost its capability-mirroring rows and this cross-check is vacuous", checked)
	}
}
