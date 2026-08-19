package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/jobs"
)

// The bug this pins: Go's flag package stops at the first positional, so
// `jobs enable <id> --hub-port 18897 --token x` parsed NO flags and quietly
// used the default hub — a command that installs and runs argv, aimed at the
// wrong machine, with no error. Flags must bind wherever the user puts them.
func TestSplitPositionalsAcceptsFlagsOnEitherSide(t *testing.T) {
	cases := []struct {
		name  string
		args  []string
		flags []string
		pos   []string
	}{
		{
			"flags after the id (the broken case)",
			[]string{"abc123", "--hub-port", "18897", "--token", "t"},
			[]string{"--hub-port", "18897", "--token", "t"},
			[]string{"abc123"},
		},
		{
			"flags before the id",
			[]string{"--hub-port", "18897", "abc123"},
			[]string{"--hub-port", "18897"},
			[]string{"abc123"},
		},
		{
			"inline values keep their own value",
			[]string{"--hub-port=18897", "abc123", "--json"},
			[]string{"--hub-port=18897", "--json"},
			[]string{"abc123"},
		},
		{
			"a bare boolean does not swallow the id",
			[]string{"--json", "abc123"},
			[]string{"--json"},
			[]string{"abc123"},
		},
		{
			"everything after -- is positional",
			[]string{"--json", "--", "-weird-id"},
			[]string{"--json"},
			[]string{"-weird-id"},
		},
		{
			"stdin's lone dash is a value, not a flag",
			[]string{"-f", "-", "--json"},
			[]string{"-f", "-", "--json"},
			nil,
		},
	}
	for _, tc := range cases {
		flags, pos := splitPositionals(tc.args, jobsValueFlags)
		if !reflect.DeepEqual(flags, tc.flags) {
			t.Errorf("%s: flags = %v, want %v", tc.name, flags, tc.flags)
		}
		if !reflect.DeepEqual(pos, tc.pos) {
			t.Errorf("%s: positional = %v, want %v", tc.name, pos, tc.pos)
		}
	}
}

// A spec is refused (with the hub's own wording) before it ever becomes a bus
// round-trip, so a typo in a file doesn't read as a connection problem.
func TestReadSpecValidatesLocally(t *testing.T) {
	dir := t.TempDir()
	write := func(name string, v any) string {
		p := filepath.Join(dir, name)
		raw, _ := json.Marshal(v)
		if err := os.WriteFile(p, raw, 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}

	good := write("good.json", map[string]any{
		"name": "Nightly", "enabled": true,
		"trigger": map[string]any{"kind": "daily", "at": "07:00"},
		"action": map[string]any{"kind": "spawn", "spawn": map[string]any{
			"cwd": "/repo", "prompt": "go",
			"context": []any{map[string]any{
				"kind": "shell", "shell": map[string]any{"command": "git status"}, "skipIfEmpty": true,
			}},
		}},
	})
	j, err := readSpec(good)
	if err != nil {
		t.Fatalf("valid spec refused: %v", err)
	}
	if len(j.Action.Spawn.Context) != 1 || !j.Action.Spawn.Context[0].SkipIfEmpty {
		t.Errorf("context step lost in the round trip: %+v", j.Action.Spawn)
	}

	bad := write("bad.json", map[string]any{
		"name": "Sneaky", "enabled": true,
		"trigger": map[string]any{"kind": "manual"},
		"action":  map[string]any{"kind": "call", "call": map[string]any{"method": "jobs.run"}},
	})
	if _, err := readSpec(bad); err == nil {
		t.Error("a call action targeting jobs.* must be refused before dialling")
	}

	if _, err := readSpec(filepath.Join(dir, "nope.json")); err == nil {
		t.Error("a missing file must be an error")
	}
}

// The summary lines are what someone reads before approving argv, so they must
// name the things that decide whether it's safe: the schedule, the action, and
// loudly, that a row is an unapproved agent proposal.
func TestJobLinesDescribeWhatWillRun(t *testing.T) {
	spawn := jobs.Job{
		Name:    "Triage",
		Trigger: jobs.Trigger{Kind: "daily", At: "09:00", Days: []int{1, 5}},
		Action: jobs.Action{Kind: "spawn", Spawn: &jobs.SpawnAction{
			Cwd: "/repo", Prompt: "p",
			Context: []jobs.ContextStep{{Kind: "shell", Shell: &jobs.ShellAction{Command: "true"}}},
		}},
	}
	if got := triggerLine(spawn); got != "daily 09:00 · Mon Fri" {
		t.Errorf("triggerLine = %q", got)
	}
	if got := actionLine(spawn); got != "1 step → agent in /repo" {
		t.Errorf("actionLine = %q", got)
	}
	if got := triggerLine(jobs.Job{Trigger: jobs.Trigger{Kind: "interval", EveryMinutes: 120}}); got != "every 2h" {
		t.Errorf("interval hours: %q", got)
	}
	if got := triggerLine(jobs.Job{Trigger: jobs.Trigger{Kind: "interval", EveryMinutes: 45}}); got != "every 45m" {
		t.Errorf("interval minutes: %q", got)
	}

	proposal := jobView{Job: jobs.Job{ID: "abc123", ProposedBy: "triage-agent", Enabled: false}}
	line := stateLine(proposal)
	for _, want := range []string{"PROPOSAL", "triage-agent", "approve abc123"} {
		if !strings.Contains(line, want) {
			t.Errorf("stateLine for a proposal must mention %q, got %q", want, line)
		}
	}

	live := jobView{Job: jobs.Job{Enabled: true}, NextRunAt: 1_800_000_000_000,
		LastRun: &jobs.Run{Status: "skipped", Detail: "context step 1: no output"}}
	if got := stateLine(live); !strings.Contains(got, "next ") || !strings.Contains(got, "last skipped") {
		t.Errorf("stateLine for a live job = %q", got)
	}
}
