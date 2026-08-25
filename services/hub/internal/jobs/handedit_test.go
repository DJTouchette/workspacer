package jobs

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

// Hand-editing jobs.json is a feature with a person on the other end of it, so
// these tests write the file the way a person does — bytes into the real path,
// from outside the service — and then ask the running service what it thinks.
// Nothing here reaches for an internal loader.
//
// Two of these fail against the pre-reload service, which is the point:
// TestHandAddedJobAppearsWithoutARestart (the edit was invisible) and
// TestHubWriteDoesNotClobberAnExternalEdit (the edit was overwritten).

// writeSpecFile puts raw JSON at the service's spec path, the way an editor
// does: whole-file replace, no help from the service.
func writeSpecFile(t *testing.T, s *Service, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(s.path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(s.path, []byte(body), 0o600); err != nil {
		t.Fatalf("write spec file: %v", err)
	}
}

// readSpecFile returns the jobs currently on disk, parsed the way any other
// reader of the file would parse them.
func readSpecFile(t *testing.T, s *Service) []Job {
	t.Helper()
	raw, err := os.ReadFile(s.path)
	if err != nil {
		t.Fatalf("read spec file: %v", err)
	}
	var doc struct {
		Jobs []Job `json:"jobs"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		t.Fatalf("spec file is not valid JSON: %v\n%s", err, raw)
	}
	return doc.Jobs
}

// listed is what jobs.list reports, which is what every UI shows.
func listed(t *testing.T, s *Service) []JobView {
	t.Helper()
	res, err := s.List(nil)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	return res.(map[string]any)["jobs"].([]JobView)
}

func viewNamed(views []JobView, name string) *JobView {
	for i := range views {
		if views[i].Name == name {
			return &views[i]
		}
	}
	return nil
}

func nextAtOf(s *Service, id string) (time.Time, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	at, ok := s.nextAt[id]
	return at, ok
}

// A job typed into the file by hand, with no id and no timestamps because a
// person does not write those, must become a scheduled job on the next tick.
func TestHandAddedJobAppearsWithoutARestart(t *testing.T) {
	s := newTestService(t, &fakeRunner{})

	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "name": "typed by hand",
	      "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 45 },
	      "action": { "kind": "shell", "shell": { "command": "echo hello" } }
	    }
	  ]
	}`)

	s.tick()

	views := listed(t, s)
	v := viewNamed(views, "typed by hand")
	if v == nil {
		t.Fatalf("the hand-added job never showed up: %+v", views)
	}
	if v.ID == "" {
		t.Error("a hand-written row has no id; the hub must mint one")
	}
	if v.CreatedAt == 0 || v.UpdatedAt == 0 {
		t.Errorf("hub-managed stamps were left blank: %+v", v.Job)
	}
	if v.NextRunAt == 0 {
		t.Error("the job appeared but was never scheduled — it would never fire")
	}
	// The minted id has to reach the file, or the next reload mints another one
	// and the job's next run walks forward forever.
	onDisk := readSpecFile(t, s)
	if len(onDisk) != 1 || onDisk[0].ID != v.ID {
		t.Errorf("the minted id was not written back: %+v", onDisk)
	}
}

// The clobber half: a hub write must apply on top of the file, not on top of
// whatever the hub happened to be holding.
func TestHubWriteDoesNotClobberAnExternalEdit(t *testing.T) {
	s := newTestService(t, &fakeRunner{})

	// One job installed the ordinary way.
	fromUI := upsert(t, s, validJob("saved from settings"))

	// A person adds a second one by hand, keeping the first.
	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "id": "`+fromUI.ID+`",
	      "name": "saved from settings",
	      "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 30 },
	      "action": { "kind": "shell", "shell": { "command": "true" } },
	      "createdAt": `+itoa(fromUI.CreatedAt)+`,
	      "updatedAt": `+itoa(fromUI.UpdatedAt)+`
	    },
	    {
	      "id": "hand-written-one",
	      "name": "added by hand",
	      "enabled": true,
	      "trigger": { "kind": "daily", "at": "03:00" },
	      "action": { "kind": "shell", "shell": { "command": "backup.sh" } }
	    }
	  ]
	}`)

	// Now the hub writes, for an unrelated reason: someone renames the first
	// job in Settings.
	fromUI.Name = "renamed in settings"
	upsert(t, s, fromUI)

	onDisk := readSpecFile(t, s)
	if len(onDisk) != 2 {
		t.Fatalf("a hub write dropped the hand-added job: %d job(s) left on disk: %+v", len(onDisk), onDisk)
	}
	var hand *Job
	for i := range onDisk {
		if onDisk[i].ID == "hand-written-one" {
			hand = &onDisk[i]
		}
	}
	if hand == nil {
		t.Fatalf("the hand-added job is gone from disk: %+v", onDisk)
	}
	if hand.Name != "added by hand" || hand.Trigger.At != "03:00" {
		t.Errorf("the hand-added job came back changed: %+v", *hand)
	}
	if v := viewNamed(listed(t, s), "renamed in settings"); v == nil {
		t.Error("the rename itself was lost")
	}
}

// A half-typed edit must cost nothing. The running schedule stays exactly as it
// was, and the machine keeps doing its work until the file parses again.
func TestMalformedHandEditKeepsTheRunningSchedule(t *testing.T) {
	s := newTestService(t, &fakeRunner{})
	j := upsert(t, s, validJob("nightly"))
	before, ok := nextAtOf(s, j.ID)
	if !ok {
		t.Fatal("the job was never scheduled to begin with")
	}

	writeSpecFile(t, s, `{ "jobs": [ { "name": "half typ`)
	s.tick()

	views := listed(t, s)
	if len(views) != 1 || views[0].ID != j.ID {
		t.Fatalf("a broken file wiped the schedule: %+v", views)
	}
	after, ok := nextAtOf(s, j.ID)
	if !ok || !after.Equal(before) {
		t.Errorf("a broken file moved the next run: %v -> %v (present=%v)", before, after, ok)
	}

	// And it recovers: the moment the file parses again, that is the truth.
	writeSpecFile(t, s, `{"jobs":[]}`)
	s.tick()
	if views := listed(t, s); len(views) != 0 {
		t.Fatalf("an explicit empty jobs array should clear the schedule: %+v", views)
	}
}

// The same rule for a file that is not there at all. Editors unlink during
// their own atomic saves and backup tools move files; neither is an instruction
// to disarm the machine.
func TestVanishedSpecFileKeepsTheRunningSchedule(t *testing.T) {
	s := newTestService(t, &fakeRunner{})
	j := upsert(t, s, validJob("nightly"))

	if err := os.Remove(s.path); err != nil {
		t.Fatalf("remove: %v", err)
	}
	s.tick()

	views := listed(t, s)
	if len(views) != 1 || views[0].ID != j.ID {
		t.Fatalf("a missing file wiped the schedule: %+v", views)
	}
}

// One bad row is one bad row. It is dropped with a log line and the others keep
// running, which is what boot has always done.
func TestOneInvalidRowIsDroppedAndTheRestSurvive(t *testing.T) {
	s := newTestService(t, &fakeRunner{})

	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "id": "good", "name": "fine", "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 30 },
	      "action": { "kind": "shell", "shell": { "command": "true" } }
	    },
	    {
	      "id": "bad", "name": "broken trigger", "enabled": true,
	      "trigger": { "kind": "daily", "at": "not a time" },
	      "action": { "kind": "shell", "shell": { "command": "true" } }
	    }
	  ]
	}`)
	s.tick()

	views := listed(t, s)
	if len(views) != 1 || views[0].ID != "good" {
		t.Fatalf("expected only the valid row to survive: %+v", views)
	}
}

// Editing one job must not restart the clock on the others. Rescheduling
// everything on every reload would mean that renaming a job at 4pm quietly
// pushed every hourly job on the machine to 5pm.
func TestEditingOneJobLeavesTheOthersAnchored(t *testing.T) {
	s := newTestService(t, &fakeRunner{})
	a := upsert(t, s, validJob("job a"))
	b := upsert(t, s, validJob("job b"))
	beforeA, _ := nextAtOf(s, a.ID)
	beforeB, _ := nextAtOf(s, b.ID)

	// Pretend some time has passed, so a re-anchor would be visible.
	later := time.Now().Add(10 * time.Minute)
	s.now = func() time.Time { return later }

	// First: a rename, which changes nothing about when anything fires.
	writeSpecFile(t, s, handEditedPair(a, b, "job a RENAMED", 30))
	s.tick()

	if v := viewNamed(listed(t, s), "job a RENAMED"); v == nil {
		t.Fatal("the rename was not picked up")
	}
	if at, ok := nextAtOf(s, a.ID); !ok || !at.Equal(beforeA) {
		t.Errorf("renaming a job moved its own next run: %v -> %v", beforeA, at)
	}
	if at, ok := nextAtOf(s, b.ID); !ok || !at.Equal(beforeB) {
		t.Errorf("renaming job a moved job b: %v -> %v", beforeB, at)
	}

	// Then: a real trigger change, which re-anchors that job and only that job.
	writeSpecFile(t, s, handEditedPair(a, b, "job a RENAMED", 90))
	s.tick()

	afterA, ok := nextAtOf(s, a.ID)
	if !ok {
		t.Fatal("job a lost its schedule")
	}
	if want := later.Add(90 * time.Minute); !afterA.Equal(want) {
		t.Errorf("edited trigger did not re-anchor: got %v, want %v", afterA, want)
	}
	if at, ok := nextAtOf(s, b.ID); !ok || !at.Equal(beforeB) {
		t.Errorf("editing job a's trigger moved job b: %v -> %v", beforeB, at)
	}
}

// handEditedPair writes the two jobs back out the way a person would, with job
// a's name and interval as given.
func handEditedPair(a, b Job, aName string, aEvery int) string {
	return `{
	  "jobs": [
	    {
	      "id": "` + a.ID + `", "name": "` + aName + `", "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": ` + strconv.Itoa(aEvery) + ` },
	      "action": { "kind": "shell", "shell": { "command": "true" } },
	      "createdAt": ` + itoa(a.CreatedAt) + `, "updatedAt": ` + itoa(a.UpdatedAt) + `
	    },
	    {
	      "id": "` + b.ID + `", "name": "job b", "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 30 },
	      "action": { "kind": "shell", "shell": { "command": "true" } },
	      "createdAt": ` + itoa(b.CreatedAt) + `, "updatedAt": ` + itoa(b.UpdatedAt) + `
	    }
	  ]
	}`
}

// Copy-pasting a job block in an editor is the obvious way to write a second
// job, and it leaves two rows sharing one id. Shared ids mean one schedule slot
// and one delete for two jobs, so they get split apart on load.
func TestDuplicateIdsInAHandEditedFileAreSplit(t *testing.T) {
	s := newTestService(t, &fakeRunner{})

	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "id": "same", "name": "first", "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 30 },
	      "action": { "kind": "shell", "shell": { "command": "true" } }
	    },
	    {
	      "id": "same", "name": "second", "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 60 },
	      "action": { "kind": "shell", "shell": { "command": "true" } }
	    }
	  ]
	}`)
	s.tick()

	views := listed(t, s)
	if len(views) != 2 {
		t.Fatalf("expected both rows: %+v", views)
	}
	if views[0].ID == views[1].ID {
		t.Fatalf("duplicate ids survived: %q", views[0].ID)
	}
	for _, v := range views {
		if v.NextRunAt == 0 {
			t.Errorf("%q was not scheduled", v.Name)
		}
	}
	if onDisk := readSpecFile(t, s); onDisk[0].ID == onDisk[1].ID {
		t.Error("the split was not written back, so it happens again every reload")
	}
}

// A hand-edited proposal stays disarmed. This is the existing rule; it is
// re-asserted through the reload path because that is a new way to reach it.
func TestHandArmingAProposalStillDoesNotSchedule(t *testing.T) {
	s := newTestService(t, &fakeRunner{})

	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "id": "p1", "name": "proposed", "enabled": true, "proposedBy": "some agent",
	      "trigger": { "kind": "interval", "everyMinutes": 5 },
	      "action": { "kind": "shell", "shell": { "command": "true" } }
	    }
	  ]
	}`)
	s.tick()

	views := listed(t, s)
	if len(views) != 1 {
		t.Fatalf("expected the row to load: %+v", views)
	}
	if views[0].NextRunAt != 0 {
		t.Error("hand-setting enabled:true on a proposal must not schedule it")
	}
}

// jobs.run has to see the file too, or "write a job, run it now" needs a wait.
func TestRunNowSeesAJobAddedByHand(t *testing.T) {
	r := &fakeRunner{}
	s := newTestService(t, r)

	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "id": "manual-one", "name": "on demand", "enabled": true,
	      "trigger": { "kind": "manual" },
	      "action": { "kind": "shell", "shell": { "command": "echo ran" } }
	    }
	  ]
	}`)

	if _, err := s.RunNow(json.RawMessage(`{"id":"manual-one"}`)); err != nil {
		t.Fatalf("run now: %v", err)
	}
	waitFor(t, func() bool { return lastRunOf(s, "manual-one") != nil })
	if got := r.recorded(); len(got) == 0 || !strings.Contains(got[0], "echo ran") {
		t.Fatalf("the hand-written command did not run: %v", got)
	}
}

// itoa keeps the JSON literals above readable.
func itoa(v int64) string { return strconv.FormatInt(v, 10) }

// The service must recognise its own writes. Without that, every save would
// look like an external edit on the next poll, and the file would be re-parsed
// thirty times an hour for nothing.
func TestTheServiceDoesNotSeeItsOwnWriteAsAnExternalEdit(t *testing.T) {
	s := newTestService(t, &fakeRunner{})
	upsert(t, s, validJob("saved by the hub"))

	s.mu.Lock()
	reloaded := s.reloadIfChangedLocked()
	s.mu.Unlock()
	if reloaded {
		t.Error("the hub's own write was reported as someone else's edit")
	}

	// A real edit, on the other hand, is one.
	writeSpecFile(t, s, `{"jobs":[]}`)
	s.mu.Lock()
	reloaded = s.reloadIfChangedLocked()
	s.mu.Unlock()
	if !reloaded {
		t.Error("an external edit was not noticed")
	}
}
