package nodes

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// ---------------------------------------------------------------------------
// THE NODE PAYLOAD CONTRACT — the producer's half.
//
// contracts/node-view-cases.json is loaded by three languages: here (the side
// that WRITES the payload), apps/desktop/src/renderer/tests/remoteNodes.test.ts
// and a #[test] in apps/tui/src/nodes.rs. Nothing pinned the five readers of
// NodeView to each other before it, and the cost of that showed up the first
// time the shape moved: the sleep path added a fifth state and two wire fields
// and reached three of the four clients.
//
// What this file owns is what the HUB decides — the state vocabulary and the
// `signal-` verdict — plus one thing no other loader can reach: whether the
// inline client in cmd/hub/mobile.html got the same words.
// ---------------------------------------------------------------------------

var nodeViewFixtureName = []string{"contracts", "node-view-cases.json"}

type nodeViewFixture struct {
	States []struct {
		State        string `json:"state"`
		Transitional bool   `json:"transitional"`
		WakeOffered  bool   `json:"wakeOffered"`
		Why          string `json:"why"`
	} `json:"states"`
	UnknownStates []string `json:"unknownStates"`
	LastExit      struct {
		CleanPrefix string `json:"cleanPrefix"`
		Cases       []struct {
			Name         string `json:"name"`
			Reason       string `json:"reason"`
			ExitCode     *int   `json:"exitCode"`
			At           string `json:"at"`
			Clean        bool   `json:"clean"`
			Notice       bool   `json:"notice"`
			RecordAbsent bool   `json:"recordAbsent"`
			Why          string `json:"why"`
		} `json:"cases"`
	} `json:"lastExit"`
	Presentation struct {
		Cases []struct {
			State             string  `json:"state"`
			Tone              string  `json:"tone"`
			Progress          bool    `json:"progress"`
			DesktopLabel      string  `json:"desktopLabel"`
			MobileLabel       string  `json:"mobileLabel"`
			FallbackDetail    string  `json:"fallbackDetail"`
			TUILabel          *string `json:"tuiLabel"`
			TUIMarker         *string `json:"tuiMarker"`
			TUIFallbackDetail *string `json:"tuiFallbackDetail"`
			Why               string  `json:"why"`
		} `json:"cases"`
	} `json:"presentation"`
}

// loadNodeViewFixture reads the corpus through sweepguard -> extinput rather
// than os.ReadFile, for the reason internal/bus/policy_test.go documents at
// length: contracts/ is four levels ABOVE this module, and cmd/go drops every
// test input whose path fails search.InDir(name, pkg.Root). Read straight, the
// whole sweep below would report `ok (cached)` after the fixture changed.
func loadNodeViewFixture(t *testing.T) nodeViewFixture {
	t.Helper()
	raw, err := sweepguard.ReadRepoFile(nodeViewFixtureName...)
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout, so the shared node-view corpus has nothing to read: %v", err)
		}
		t.Fatalf("%v", err)
	}
	var fx nodeViewFixture
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatalf("parse %s: %v", filepath.ToSlash(filepath.Join(nodeViewFixtureName...)), err)
	}
	if len(fx.States) == 0 || len(fx.LastExit.Cases) == 0 || len(fx.Presentation.Cases) == 0 {
		t.Fatal("the node-view corpus decoded to empty blocks — a field was renamed and this sweep is asserting nothing")
	}
	return fx
}

// TestTheStateVocabularyMatchesTheContract closes the vocabulary in BOTH
// directions, and the second one is the half that matters.
//
// "every state in the fixture is Valid()" alone would pass a package that had
// quietly grown a sixth state no client has heard of — which is exactly what
// happened with `stopping`. So the package's own consts are read out of
// nodes.go and the two sets must be equal.
func TestTheStateVocabularyMatchesTheContract(t *testing.T) {
	fx := loadNodeViewFixture(t)

	var fromFixture []string
	for _, c := range fx.States {
		if !State(c.State).Valid() {
			t.Errorf("the contract declares the state %q and State.Valid() rejects it — a client is being held to a vocabulary the hub will never send", c.State)
		}
		fromFixture = append(fromFixture, c.State)
	}
	sort.Strings(fromFixture)

	declared := declaredStateConsts(t)
	if strings.Join(declared, ",") != strings.Join(fromFixture, ",") {
		t.Errorf("the State consts in nodes.go are %v and contracts/node-view-cases.json declares %v — a state the hub can send and the contract does not name is one every client coerces to `unreachable`: the warning tone, the wrong verb, and (for a state the machine is passing THROUGH) an offer to start a machine that is already on its way somewhere",
			declared, fromFixture)
	}

	// The coercion target has to be in the vocabulary, or "render an unknown
	// state as unreachable" names something a client cannot render.
	if !State("unreachable").Valid() {
		t.Error("`unreachable` is the mandated coercion target for an unrecognised state and the hub does not consider it a state")
	}

	for _, s := range fx.UnknownStates {
		if State(s).Valid() {
			t.Errorf("the contract lists %q among the strings a client must coerce to `unreachable`, and the hub considers it a real state", s)
		}
	}
}

// declaredStateConsts reads the State consts straight out of the package's own
// source. A hand-kept list here would be a third copy of the vocabulary, which
// is the defect this file exists to close.
func declaredStateConsts(t *testing.T) []string {
	t.Helper()
	raw, err := os.ReadFile("nodes.go")
	if err != nil {
		t.Fatalf("read nodes.go to enumerate the State consts: %v", err)
	}
	re := regexp.MustCompile(`(?m)^\s*State\w+\s+State\s*=\s*"([a-z-]+)"`)
	var out []string
	for _, m := range re.FindAllStringSubmatch(string(raw), -1) {
		out = append(out, m[1])
	}
	if len(out) == 0 {
		t.Fatal("found no `StateX State = \"…\"` consts in nodes.go — the regex broke, and this guard would report agreement it never checked")
	}
	sort.Strings(out)
	return out
}

// TestTheExitRecordVerdictsMatchTheContract is the `signal-` prefix rule, which
// is the sharpest edge on this payload: the reason string is produced by a
// shell printf on the node and re-parsed by four independent startsWith checks.
func TestTheExitRecordVerdictsMatchTheContract(t *testing.T) {
	fx := loadNodeViewFixture(t)
	if fx.LastExit.CleanPrefix != "signal-" {
		t.Fatalf("the contract's cleanPrefix is %q; the hub's Clean() tests for %q", fx.LastExit.CleanPrefix, "signal-")
	}
	var cleans, crashes int
	for _, c := range fx.LastExit.Cases {
		var rec *ExitRecord
		if !c.RecordAbsent {
			rec = &ExitRecord{Reason: c.Reason, ExitCode: c.ExitCode, At: c.At}
		}
		if got := rec.Clean(); got != c.Clean {
			t.Errorf("%s: Clean() = %v, want %v — reason %q", c.Name, got, c.Clean, c.Reason)
		}
		// Describe() is what a client sees when the hub writes the detail
		// itself, so the notice column has to hold on this side too: a
		// sentence for the endings worth reporting, silence for the rest.
		notice := !rec.Clean() && rec.Describe() != ""
		if notice != c.Notice {
			t.Errorf("%s: the hub %s a crash sentence (Describe() = %q), and the contract says %v", c.Name,
				map[bool]string{true: "writes", false: "writes no"}[notice], rec.Describe(), c.Notice)
		}
		if c.Clean {
			cleans++
		}
		if c.Notice {
			crashes++
		}
	}
	// A corpus that drifted to all-clean or all-crash would still pass every
	// assertion above while pinning only one arm of the rule.
	if cleans == 0 || crashes == 0 {
		t.Fatalf("the corpus exercises %d clean and %d crash endings — one arm of the verdict is unpinned", cleans, crashes)
	}
}

// TestEveryContractStateReachedTheMobileClient is the one check no other loader
// can make.
//
// /m's node client is inline JavaScript inside a go:embed'ed HTML file, so it
// has no test harness of its own and no import anything could stub. It is also
// the client most likely to be forgotten — it was the one place the sleep
// path's `stopping` DID reach, and that was luck rather than a mechanism.
//
// This is deliberately a substring check and not a parse: it asserts the words
// are in the file, which is the whole failure mode (a state key absent from
// NODE_META falls through to the `unreachable` entry — amber, "Can't reach" —
// on a machine the user just told to stop).
func TestEveryContractStateReachedTheMobileClient(t *testing.T) {
	fx := loadNodeViewFixture(t)
	raw, err := os.ReadFile(filepath.Join("..", "..", "cmd", "hub", "mobile.html"))
	if err != nil {
		t.Fatalf("read cmd/hub/mobile.html: %v", err)
	}
	html := string(raw)
	for _, c := range fx.Presentation.Cases {
		if !strings.Contains(html, c.State+":") {
			t.Errorf("cmd/hub/mobile.html has no NODE_META entry for the state %q — /m will paint it amber and call it \"Can't reach\"", c.State)
		}
		if !strings.Contains(html, c.MobileLabel) {
			t.Errorf("cmd/hub/mobile.html does not carry the chip label %q for %q", c.MobileLabel, c.State)
		}
		if !strings.Contains(html, c.FallbackDetail) {
			t.Errorf("cmd/hub/mobile.html does not carry the fallback sentence for %q verbatim (%q) — this string is byte-identical on the desktop and /m by contract", c.State, c.FallbackDetail)
		}
	}
}

// TestTheViewCarriesEveryContractState is the producer's own end: ViewOf must
// put each state on the wire as the string the contract names, unchanged. It is
// a small assertion and it is the seam where a typo would reach every client at
// once.
func TestTheViewCarriesEveryContractState(t *testing.T) {
	fx := loadNodeViewFixture(t)
	n := Node{ID: "ord", Label: "ord label"}
	for _, c := range fx.States {
		v := ViewOf(n, State(c.State), time.Time{}, time.Time{}, "", 0)
		if v.State != c.State {
			t.Errorf("ViewOf put %q on the wire for the state %q", v.State, c.State)
		}
	}
}
