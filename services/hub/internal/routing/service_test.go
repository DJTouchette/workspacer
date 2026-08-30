package routing

import (
	"bytes"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func matrixPath(t *testing.T) string {
	t.Helper()
	return filepath.Join(t.TempDir(), "workspacer-hub", "routing.yaml")
}

func write(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

// --- seeding -----------------------------------------------------------------

// TestSeedWritesTheShippedDefaultVerbatim — a file the user is told to edit has
// to exist, and it has to arrive with its comments, because those comments are
// the documentation for the policy.
func TestSeedWritesTheShippedDefaultVerbatim(t *testing.T) {
	path := matrixPath(t)
	s := New(path, nil)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("first run did not write the default matrix: %v", err)
	}
	if !bytes.Equal(raw, DefaultBytes()) {
		t.Error("the seeded file is not the embedded bytes verbatim")
	}
	if !strings.Contains(string(raw), "THE SHIPPED DEFAULT ROUTING MATRIX") {
		t.Error("the seeded file lost its header — the sentence that tells the user it will not be rewritten")
	}
	if !strings.Contains(string(raw), "# ---") {
		t.Error("the seeded file lost its comments, which are the only documentation of this policy the user gets")
	}
	if runtime.GOOS != "windows" {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		if perm := info.Mode().Perm(); perm != 0o600 {
			t.Errorf("mode %o, want 0600 — this file decides how much capability and autonomy a spawned agent gets", perm)
		}
	}
	if s.Matrix() == nil || s.Matrix().Version != 1 {
		t.Errorf("the seeded file did not load back: %+v", s.Matrix())
	}
}

// TestSeedDoesNotOverwriteAnExistingFile — and records the offer anyway, so a
// pre-marker install still honours a later delete.
func TestSeedDoesNotOverwriteAnExistingFile(t *testing.T) {
	path := matrixPath(t)
	write(t, path, "version: 1\nactive_profile: codex_only\n")
	New(path, nil)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "THE SHIPPED DEFAULT") {
		t.Fatal("an existing routing.yaml was overwritten with the shipped default")
	}
	if _, err := os.Stat(seedMarkerFor(path)); err != nil {
		t.Errorf("a pre-marker install was not recorded, so a later delete would be undone: %v", err)
	}
}

// TestAFileDeletedOnPurposeStaysDeleted is the seedGlobalStarters discipline:
// seed once, and never argue with the user about it afterwards.
func TestAFileDeletedOnPurposeStaysDeleted(t *testing.T) {
	path := matrixPath(t)
	New(path, nil)
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	s := New(path, nil)
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatal("the deleted file was written back on the next boot")
	}
	// And deleting it costs nothing: the compiled-in copy is still the base of
	// every answer.
	r, err := s.ResolveOrDie(t, "implementer")
	if err != nil {
		t.Fatalf("with no file at all, roles must still resolve: %v", err)
	}
	if r.Assignment.Model == "" {
		t.Errorf("resolved to an empty assignment: %+v", r)
	}
}

// ResolveOrDie is a tiny helper so the test above reads in one line.
func (s *Service) ResolveOrDie(t *testing.T, role string) (Resolved, error) {
	t.Helper()
	m := s.Matrix()
	if m == nil {
		t.Fatal("no matrix loaded at all")
	}
	return m.ResolveRole(role)
}

// TestADisabledPathRunsOnTheCompiledInDefaults — path "" is the read-only /
// test deployment, and it must be a complete matrix, not an empty one.
func TestADisabledPathRunsOnTheCompiledInDefaults(t *testing.T) {
	s := New("", nil)
	m := s.Matrix()
	if m == nil || len(m.Roles) == 0 {
		t.Fatalf("no matrix with path disabled: %+v", m)
	}
	if s.ReloadIfChanged() {
		t.Error("a disabled path reported a reload")
	}
}

// --- hand editing ------------------------------------------------------------

// TestAnEditIsPickedUpOnTheNextTick is the whole hand-editing story: open the
// file, save it, no restart.
func TestAnEditIsPickedUpOnTheNextTick(t *testing.T) {
	path := matrixPath(t)
	write(t, path, "active_profile: mixed\n")
	s := New(path, nil)
	if got, _ := s.Matrix().ActiveProfileName(); got != "mixed" {
		t.Fatalf("initial profile %q", got)
	}
	if s.ReloadIfChanged() {
		t.Error("an untouched file reported a change — the poll compares CONTENT, and re-reading the same bytes is not an edit")
	}

	write(t, path, "active_profile: anthropic_only\n")
	if !s.ReloadIfChanged() {
		t.Fatal("a saved edit was not picked up")
	}
	if got, _ := s.Matrix().ActiveProfileName(); got != "anthropic_only" {
		t.Errorf("profile after the edit = %q", got)
	}
	// And the value arrives all the way through to the consumer.
	r, err := s.Matrix().ResolveRole("implementer")
	if err != nil {
		t.Fatal(err)
	}
	if r.Assignment.Provider != "claude" || r.Assignment.Model != "opus" {
		t.Errorf("implementer resolved to %+v, want the anthropic_only frontier the edited file selects", r.Assignment)
	}
}

// TestABrokenEditLeavesTheRunningMatrixExactlyAsItWas.
//
// A half-typed edit, an editor unlinking the file during its own atomic save, or
// a backup tool moving it must not silently disarm routing.
func TestABrokenEditLeavesTheRunningMatrixExactlyAsItWas(t *testing.T) {
	path := matrixPath(t)
	write(t, path, "active_profile: codex_only\n")
	s := New(path, nil)
	before, _ := s.Matrix().ActiveProfileName()

	// 1. Mid-save garbage.
	write(t, path, "active_profile: [codex_only\n  broken: {{{\n")
	if s.ReloadIfChanged() {
		t.Error("a document that does not parse was applied")
	}
	if got, _ := s.Matrix().ActiveProfileName(); got != before {
		t.Errorf("profile moved to %q on a broken parse, want %q held", got, before)
	}

	// 2. The file vanishes for a moment.
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if s.ReloadIfChanged() {
		t.Error("an unreadable file was treated as a change")
	}
	if got, _ := s.Matrix().ActiveProfileName(); got != before {
		t.Errorf("an unlinked file disarmed the matrix: profile %q", got)
	}

	// 3. It comes back, and the edit lands.
	write(t, path, "active_profile: anthropic_only\n")
	if !s.ReloadIfChanged() {
		t.Fatal("the file coming back was not picked up")
	}
	if got, _ := s.Matrix().ActiveProfileName(); got != "anthropic_only" {
		t.Errorf("profile after recovery = %q", got)
	}
}

// TestAnEmptyDocumentIsNotAWipe — a 0-byte, whitespace-only or comment-only file
// parses to a document with no keys, which means "no overrides", not "no matrix".
func TestAnEmptyDocumentIsNotAWipe(t *testing.T) {
	for _, body := range []string{"", "   \n\n", "# I deleted everything\n"} {
		path := matrixPath(t)
		write(t, path, body)
		s := New(path, nil)
		m := s.Matrix()
		if m == nil || len(m.Roles) == 0 || len(m.Profiles) == 0 {
			t.Fatalf("%q left no matrix: %+v", body, m)
		}
		if r, err := m.ResolveRole("judge"); err != nil || r.Assignment.Model == "" {
			t.Errorf("%q: judge did not resolve: %+v %v", body, r, err)
		}
	}
}

// --- the trace ---------------------------------------------------------------

// TestAValueOnDiskReachesAConsumer is the end-to-end proof that this setting is
// not merely written: a real file in a real hub-state-shaped directory, through
// the real seeder, the real content-hash reader, the real deep merge, out of the
// real consumer API — and every neighbouring value it did not name still there.
//
// "Written but never read" is this project's most common bug; this is the test
// that would catch it.
func TestAValueOnDiskReachesAConsumer(t *testing.T) {
	path := matrixPath(t)
	write(t, path, `# a user's hand edit
active_profile: mixed
roles:
  scout: frontier          # this shop wants a strong scout
profiles:
  mixed:
    frontier:
      effort: xhigh        # ...and its frontier turned all the way up
thresholds:
  spend_down:
    time_to_reset_minutes: 45
ceilings:
  default: { max_capability: balanced, max_tool_scope: triage }
`)
	s := New(path, nil)
	m := s.Matrix()
	if m == nil {
		t.Fatal("nothing loaded")
	}
	if m.Source != path {
		t.Errorf("Source = %q, want the file on disk", m.Source)
	}

	// 1. role -> capability -> assignment, with the user's value at each hop.
	scout, err := m.ResolveRole("scout")
	if err != nil {
		t.Fatal(err)
	}
	if scout.Capability != "frontier" {
		t.Errorf("scout capability = %q, want the user's frontier", scout.Capability)
	}
	if scout.Assignment.Effort != "xhigh" {
		t.Errorf("scout effort = %q, want the user's xhigh", scout.Assignment.Effort)
	}
	if scout.Assignment.Provider != "codex" || scout.Assignment.Model != "gpt-5.6-sol" {
		t.Errorf("the keys the user did NOT name were lost: %+v", scout.Assignment)
	}
	if scout.FellBack {
		t.Error("a value that was in the file should not have needed a fallback")
	}

	// 2. A scalar in a nested block.
	if got := m.Thresholds.SpendDown.TimeToResetMinutes; got != 45 {
		t.Errorf("spend_down.time_to_reset_minutes = %v, want the user's 45", got)
	}
	if got := m.Thresholds.SpendDown.MinRemainingPct; got != 50 {
		t.Errorf("its untouched sibling = %v, want the shipped 50", got)
	}

	// 3. The ceiling block, through its own lookup.
	c, key := m.CeilingFor(filepath.Join(string(filepath.Separator), "anywhere"))
	if key != CeilingDefaultKey || c.MaxCapability != "balanced" || c.MaxToolScope != "triage" {
		t.Errorf("ceiling = %+v at %q, want the user's balanced/triage", c, key)
	}

	// 4. The keys taken from the file are named, so nothing half-applies in
	//    silence.
	for _, want := range []string{
		"active_profile", "roles.scout", "profiles.mixed.frontier.effort",
		"thresholds.spend_down.time_to_reset_minutes",
		"ceilings.default.max_capability", "ceilings.default.max_tool_scope",
	} {
		if !contains(m.Applied, want) {
			t.Errorf("key %q applied but not reported; got %v", want, m.Applied)
		}
	}
	if len(m.Unrecognized) != 0 {
		t.Errorf("a hand edit of real keys was reported as typos: %v", m.Unrecognized)
	}
	if len(m.Issues) != 0 {
		t.Errorf("a valid hand edit produced issues: %v", m.Issues)
	}
}

// TestDefaultPathIsTheHubStateDir — the same directory as jobs.json, which is
// what the secret gate refuses and therefore what makes the ceiling enforceable.
func TestDefaultPathIsTheHubStateDir(t *testing.T) {
	p := DefaultPath()
	if filepath.Base(p) != "routing.yaml" {
		t.Errorf("DefaultPath = %q", p)
	}
	if dir := filepath.Base(filepath.Dir(p)); dir != "workspacer-hub" && p != "routing.yaml" {
		t.Errorf("DefaultPath = %q, want it beside jobs.json in workspacer-hub", p)
	}
}
