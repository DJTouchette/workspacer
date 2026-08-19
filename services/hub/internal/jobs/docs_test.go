package jobs

import (
	"encoding/json"
	"html"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"strings"
	"testing"
)

// The public docs are the ONLY spec most authors of a job ever see — a person
// writing jobs.json by hand, or an LLM asked "write me a job that…", has
// nothing else to go on. So the docs are pinned twice here:
//
//  1. every example marked `data-job-example` must survive Validate, so the
//     copy-paste path can't silently rot;
//  2. every field this package defines must be NAMED somewhere in the docs, so
//     a field added to the spec can't quietly become undocumented — which is
//     the failure that makes a model write a job missing the one key that
//     mattered.
//
// The docs live outside this module; a hub built on its own just skips.
const docsPath = "../../../../landing/docs.html"

func loadDocs(t *testing.T) string {
	t.Helper()
	raw, err := os.ReadFile(filepath.Clean(docsPath))
	if err != nil {
		t.Skipf("landing docs not present (%v) — skipping the docs pin", err)
	}
	return string(raw)
}

var exampleRe = regexp.MustCompile(`(?s)<pre data-job-example><code>(.*?)</code></pre>`)

func TestDocumentedJobExamplesAreValid(t *testing.T) {
	docs := loadDocs(t)
	matches := exampleRe.FindAllStringSubmatch(docs, -1)
	if len(matches) == 0 {
		t.Fatal("no <pre data-job-example> blocks found — did the docs markup change?")
	}
	for i, m := range matches {
		body := html.UnescapeString(m[1])
		var j Job
		if err := json.Unmarshal([]byte(body), &j); err != nil {
			t.Errorf("example %d is not valid JSON: %v\n%s", i+1, err, body)
			continue
		}
		if err := Validate(&j); err != nil {
			t.Errorf("example %d would be REFUSED by the hub: %v\n%s", i+1, err, body)
		}
	}
}

// TestEveryJobFieldIsDocumented walks the spec's json tags and asserts the docs
// name each one. Reflection rather than a hand-list: a hand-list is a second
// thing to forget.
func TestEveryJobFieldIsDocumented(t *testing.T) {
	docs := loadDocs(t)

	// Hub-managed bookkeeping — set by the service, never written by an author,
	// so naming them in an authoring reference would only invite someone (or
	// some model) to try to set them.
	skip := map[string]bool{"createdAt": true, "updatedAt": true}

	seen := map[string]bool{}
	var walk func(reflect.Type)
	walk = func(rt reflect.Type) {
		for rt.Kind() == reflect.Ptr || rt.Kind() == reflect.Slice {
			rt = rt.Elem()
		}
		if rt.Kind() != reflect.Struct || seen[rt.Name()] {
			return
		}
		seen[rt.Name()] = true
		for i := 0; i < rt.NumField(); i++ {
			f := rt.Field(i)
			name := strings.Split(f.Tag.Get("json"), ",")[0]
			if name != "" && name != "-" && !skip[name] {
				if !strings.Contains(docs, name) {
					t.Errorf("%s.%s (json %q) is part of the job spec but appears nowhere in %s — "+
						"an author reading the docs cannot know it exists",
						rt.Name(), f.Name, name, docsPath)
				}
			}
			walk(f.Type)
		}
	}
	walk(reflect.TypeOf(Job{}))
}

// TestEveryDocumentedKindIsAccepted closes the loop the other way: the enum
// values the docs tell an author to write are exactly the ones Validate takes.
func TestEveryDocumentedKindIsAccepted(t *testing.T) {
	docs := loadDocs(t)

	triggers := map[string]Trigger{
		"interval": {Kind: "interval", EveryMinutes: 60},
		"daily":    {Kind: "daily", At: "09:00", Days: []int{1, 2, 3, 4, 5}},
		"once":     {Kind: "once", Once: "2026-09-01T09:00:00Z"},
		"manual":   {Kind: "manual"},
	}
	for kind, tr := range triggers {
		if !strings.Contains(docs, kind) {
			t.Errorf("trigger kind %q is accepted but undocumented", kind)
		}
		j := validJob("doc")
		j.Trigger = tr
		if err := Validate(&j); err != nil {
			t.Errorf("documented trigger %q refused: %v", kind, err)
		}
	}

	actions := map[string]Action{
		"spawn": {Kind: "spawn", Spawn: &SpawnAction{Cwd: "/abs/path", Prompt: "p"}},
		"shell": {Kind: "shell", Shell: &ShellAction{Command: "true"}},
		"call":  {Kind: "call", Call: &CallAction{Method: "sessions.list"}},
	}
	for kind, a := range actions {
		if !strings.Contains(docs, kind) {
			t.Errorf("action kind %q is accepted but undocumented", kind)
		}
		j := validJob("doc")
		j.Action = a
		if err := Validate(&j); err != nil {
			t.Errorf("documented action %q refused: %v", kind, err)
		}
	}

	// Both context-step kinds, with every guard set the way the docs describe.
	steps := []ContextStep{
		{Kind: "shell", Shell: &ShellAction{Command: "git log --oneline -20"},
			SkipIfEmpty: true, SkipUnlessMatch: "FAIL", IgnoreExitCode: true},
		{Kind: "call", Call: &CallAction{Method: "sessions.list"}, SkipIfEmpty: true},
	}
	j := validJob("doc")
	j.Action = Action{Kind: "spawn", Spawn: &SpawnAction{Cwd: "/abs/path", Prompt: "{{output}}", Context: steps}}
	if err := Validate(&j); err != nil {
		t.Fatalf("documented context steps refused: %v", err)
	}

	// The documented caps and placeholders, in the words the docs use.
	for _, phrase := range []string{"{{output}}", "{{output.1}}", "jobs.json", "0600"} {
		if !strings.Contains(docs, phrase) {
			t.Errorf("docs no longer mention %q, which an author needs", phrase)
		}
	}
	if !strings.Contains(docs, "four context steps") && !strings.Contains(docs, "max 4") {
		t.Error("docs no longer state the context-step cap")
	}
}

// The other half of "can someone write one of these from the docs alone":
// specs in the shape a person — or a model asked in plain English — actually
// produces, using only what the page states. Each must be accepted verbatim.
// A doc change that makes a realistic spec invalid fails here, which is the
// case a fixture of hand-picked valid specs would miss.
var authored = map[string]string{
	"weekday morning agent": `{
	  "name": "Morning PR sweep", "enabled": true,
	  "trigger": {"kind":"daily","at":"08:30","days":[1,2,3,4,5]},
	  "action": {"kind":"spawn","spawn":{"cwd":"/home/me/api","prompt":"Summarize new PRs."}}
	}`,
	"minimal, no optional keys": `{
	  "name":"Ping","enabled":true,
	  "trigger":{"kind":"manual"},
	  "action":{"kind":"shell","shell":{"command":"echo hi"}}
	}`,
	"interval shell with cwd": `{
	  "name":"Prune","enabled":true,
	  "trigger":{"kind":"interval","everyMinutes":240},
	  "action":{"kind":"shell","shell":{"command":"git worktree prune","cwd":"/home/me/api"}}
	}`,
	"once, RFC3339": `{
	  "name":"Kickoff","enabled":true,
	  "trigger":{"kind":"once","once":"2026-09-01T09:00:00Z"},
	  "action":{"kind":"spawn","spawn":{"cwd":"/home/me/api","prompt":"Start the migration.","provider":"claude","model":"opus"}}
	}`,
	"guarded: only if the disk is full": `{
	  "name":"Disk pressure","enabled":true,
	  "trigger":{"kind":"interval","everyMinutes":60},
	  "action":{"kind":"spawn","spawn":{
	    "cwd":"/home/me/ops",
	    "prompt":"Disk is filling up:\n{{output}}\nFind the biggest offenders and propose cleanup.",
	    "context":[{"kind":"shell","shell":{"command":"df -h | awk '$5+0 > 90'"},"skipIfEmpty":true,"ignoreExitCode":true}]
	  }}
	}`,
	"two steps, indexed placeholders": `{
	  "name":"Nightly digest","enabled":true,
	  "trigger":{"kind":"daily","at":"22:00"},
	  "action":{"kind":"spawn","spawn":{
	    "cwd":"/home/me/api",
	    "prompt":"Commits:\n{{output.1}}\n\nSessions today:\n{{output.2}}\n\nWrite DIGEST.md.",
	    "context":[
	      {"kind":"shell","shell":{"command":"git log --oneline --since=1.day"},"skipIfEmpty":true},
	      {"kind":"call","call":{"method":"sessions.list","params":{}}}
	    ]
	  }}
	}`,
	"blank id on create": `{
	  "id":"","name":"Blank id","enabled":true,
	  "trigger":{"kind":"interval","everyMinutes":15},
	  "action":{"kind":"call","call":{"method":"notifications.post","params":{"title":"tick"}}}
	}`,
	"regex guard": `{
	  "name":"CI watch","enabled":true,
	  "trigger":{"kind":"interval","everyMinutes":30},
	  "action":{"kind":"spawn","spawn":{
	    "cwd":"/home/me/api",
	    "prompt":"CI says:\n{{output}}\nFix it.",
	    "context":[{"kind":"shell","shell":{"command":"gh run list --limit 5"},"skipUnlessMatch":"failure","ignoreExitCode":true}]
	  }}
	}`,
}

func TestSpecsWrittenFromTheDocsAreAccepted(t *testing.T) {
	for name, raw := range authored {
		var j Job
		if err := json.Unmarshal([]byte(raw), &j); err != nil {
			t.Errorf("%s: not valid JSON: %v", name, err)
			continue
		}
		if err := Validate(&j); err != nil {
			t.Errorf("%s: hub REFUSED a spec written from the docs: %v", name, err)
		}
	}
}
