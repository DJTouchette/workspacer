package main

import (
	"encoding/json"
	"reflect"
	"testing"
)

// Portable fixtures survive deleting Go. Run the identical inputs through the
// old projection while it remains the migration reference.
func TestRustMigrationSnapshotFixtures(t *testing.T) {
	raw := mustReadRepoFile(t, "contracts", "hub-snapshot-cases.json")
	var fixture struct {
		Cases []struct {
			Name     string
			Raw      json.RawMessage
			Expected any
		}
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Cases) < 4 {
		t.Fatal("missing snapshot migration fixtures")
	}
	for _, c := range fixture.Cases {
		t.Run(c.Name, func(t *testing.T) {
			var actual any
			if err := json.Unmarshal(compatSnapshot(c.Raw), &actual); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(actual, c.Expected) {
				t.Fatalf("got %#v; want %#v", actual, c.Expected)
			}
		})
	}
}
