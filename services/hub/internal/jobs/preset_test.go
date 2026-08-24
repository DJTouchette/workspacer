package jobs

import (
	"encoding/json"
	"html"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// The power-down-when-quiet job is written out twice: once in landing/docs.html
// as a documented example, once in the desktop's Settings -> Jobs as a template
// chip that pre-fills the editor. contracts/job-preset-power-down.json is the
// single thing both are held to. This file is the Go half of that pin: it runs
// the fixture through the REAL validator (the same Validate the hub applies on
// save) and checks it against the documented block. The renderer half is
// apps/desktop/src/renderer/tests/jobsPreset.test.ts, which holds the template
// chip to the same fixture.
//
// The two places the preset deliberately DIFFERS from the docs are asserted
// here as well, because they are the whole safety design and a later reader
// tidying the "inconsistency" away would arm a job that powers down a machine:
// the preset is disabled, and its script path is an obvious blank rather than
// the docs' plausible-looking /opt/wks/power-down.sh.
const presetFixturePath = "../../../../contracts/job-preset-power-down.json"

type powerDownFixture struct {
	DocsExampleName string `json:"docsExampleName"`
	QuiescenceCheck string `json:"quiescenceCheck"`
	Placeholder     string `json:"placeholder"`
	Spec            Job    `json:"spec"`
}

func loadPresetFixture(t *testing.T) powerDownFixture {
	t.Helper()
	raw, err := os.ReadFile(filepath.Clean(presetFixturePath))
	if err != nil {
		t.Skipf("contracts fixture not present (%v), skipping the preset pin", err)
	}
	var f powerDownFixture
	if err := json.Unmarshal(raw, &f); err != nil {
		t.Fatalf("fixture is not valid JSON: %v", err)
	}
	return f
}

func TestPowerDownPresetIsAcceptedAndArrivesDisabled(t *testing.T) {
	f := loadPresetFixture(t)

	spec := f.Spec
	if err := Validate(&spec); err != nil {
		t.Fatalf("the shipped preset would be REFUSED by the hub: %v", err)
	}
	if f.Spec.Enabled {
		t.Error("the preset must arrive DISABLED: its command runs a script the user has not written yet")
	}

	cmd := f.Spec.Action.Shell.Command
	if !strings.HasPrefix(cmd, f.QuiescenceCheck+" && ") {
		t.Errorf("preset command %q does not run %q first", cmd, f.QuiescenceCheck)
	}
	if !strings.Contains(cmd, f.Placeholder) {
		t.Errorf("preset command %q no longer carries the fill-in-the-blank marker %q. "+
			"a real-looking path here is one a user saves without reading", cmd, f.Placeholder)
	}
}

// The documented example and the preset must agree on everything that is not
// one of the two deliberate differences.
func TestPowerDownPresetMatchesTheDocumentedExample(t *testing.T) {
	f := loadPresetFixture(t)
	docs := loadDocs(t)

	var documented *Job
	for _, m := range exampleRe.FindAllStringSubmatch(docs, -1) {
		var j Job
		if err := json.Unmarshal([]byte(html.UnescapeString(m[1])), &j); err != nil {
			continue
		}
		if j.Name == f.DocsExampleName {
			documented = &j
			break
		}
	}
	if documented == nil {
		t.Fatalf("no <pre data-job-example> block named %q in %s. The preset in Settings -> Jobs "+
			"now has no documented counterpart", f.DocsExampleName, docsPath)
	}

	if !reflect.DeepEqual(documented.Trigger, f.Spec.Trigger) {
		t.Errorf("trigger drift: docs %+v, preset %+v", documented.Trigger, f.Spec.Trigger)
	}
	if documented.Action.Kind != f.Spec.Action.Kind {
		t.Errorf("action kind drift: docs %q, preset %q", documented.Action.Kind, f.Spec.Action.Kind)
	}
	if documented.Action.Shell == nil || f.Spec.Action.Shell == nil {
		t.Fatal("both the documented example and the preset must be shell actions")
	}
	if documented.Action.Shell.Cwd != f.Spec.Action.Shell.Cwd {
		t.Errorf("cwd drift: docs %q, preset %q", documented.Action.Shell.Cwd, f.Spec.Action.Shell.Cwd)
	}

	docCheck, docScript, ok := strings.Cut(documented.Action.Shell.Command, " && ")
	if !ok {
		t.Fatalf("documented command %q is no longer <check> && <script>", documented.Action.Shell.Command)
	}
	presetCheck, presetScript, ok := strings.Cut(f.Spec.Action.Shell.Command, " && ")
	if !ok {
		t.Fatalf("preset command %q is no longer <check> && <script>", f.Spec.Action.Shell.Command)
	}
	if docCheck != presetCheck {
		t.Errorf("the quiescence check drifted: docs run %q, the preset runs %q", docCheck, presetCheck)
	}
	if docCheck != f.QuiescenceCheck {
		t.Errorf("docs run %q, fixture says %q", docCheck, f.QuiescenceCheck)
	}

	// The differences, pinned as differences.
	if presetScript != f.Placeholder {
		t.Errorf("the preset's script slot is %q, not the blank %q", presetScript, f.Placeholder)
	}
	if docScript == f.Placeholder {
		t.Errorf("the docs now show the blank %q where a reader needs a worked example", f.Placeholder)
	}
	if !documented.Enabled {
		t.Error("the documented example shows a working job, so it stays enabled; only the preset arrives off")
	}
	if f.Spec.Enabled {
		t.Error("the preset must arrive disabled even though the documented example is enabled")
	}
}
