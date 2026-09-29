package jobs

import (
	"encoding/json"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
	"testing"
	"time"
)

func TestRustMigrationJobFixtures(t *testing.T) {
	raw, err := sweepguard.ReadRepoFile("contracts", "hub-job-cases.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		ScheduleCases []struct {
			Name          string
			Trigger       Trigger
			After         string
			OffsetSeconds int
			Expected      *string
		}
		PromptCases []struct {
			Name, Prompt string
			Outputs      []string
			Expected     string
		}
	}
	if err = json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.ScheduleCases) < 5 || len(fixture.PromptCases) < 4 {
		t.Fatal("job migration corpus was reduced")
	}
	for _, c := range fixture.ScheduleCases {
		t.Run(c.Name, func(t *testing.T) {
			after, err := time.Parse(time.RFC3339, c.After)
			if err != nil {
				t.Fatal(err)
			}
			at, ok := NextRun(c.Trigger, after, time.FixedZone("fixture", c.OffsetSeconds))
			if c.Expected == nil {
				if ok {
					t.Fatal("unexpected scheduled time")
				}
				return
			}
			if !ok || at.UTC().Format(time.RFC3339) != *c.Expected {
				t.Fatalf("got %v (%v), want %s", at, ok, *c.Expected)
			}
		})
	}
	for _, c := range fixture.PromptCases {
		t.Run(c.Name, func(t *testing.T) {
			if got := fillPrompt(c.Prompt, c.Outputs); got != c.Expected {
				t.Fatalf("got %q, want %q", got, c.Expected)
			}
		})
	}
}
