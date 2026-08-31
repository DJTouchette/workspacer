package main

// THE DRIFT GUARD FOR THE BUS'S SPAWN-KEY GATE.
//
// internal/bus refuses an agents.spawn whose top-level key case-folds to a spawn
// param it knows about without being spelled as one — the single-case-variant
// authority bypass (spawnkeys.go). That gate is only as complete as its list: a
// provider field the list does not name is a field whose case variants reach
// that provider unexamined, and the failure is INVISIBLE, because the spawn
// works.
//
// So the list is checked against the providers rather than maintained by hand.
// The headless provider is checked by REFLECTION over its own struct; the
// desktop provider is checked by reading the destructure block out of
// hubCapabilities.ts, which is the same file capspec's params detector already
// treats as the desktop's spawn surface.

import (
	"reflect"
	"regexp"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/bus"
)

func canonicalSpawnKeySet(t *testing.T) map[string]bool {
	t.Helper()
	set := map[string]bool{}
	for _, k := range bus.SpawnParamKeys() {
		set[k] = true
	}
	if len(set) == 0 {
		t.Fatal("bus.SpawnParamKeys() is empty — the gate it feeds would refuse nothing")
	}
	return set
}

// TestEverySpawnParamFieldIsInTheBusCanonicalKeySet is the reflection half. Add
// a field to spawnParams with a json tag the bus has never heard of and this
// fails, naming the tag.
func TestEverySpawnParamFieldIsInTheBusCanonicalKeySet(t *testing.T) {
	set := canonicalSpawnKeySet(t)
	rt := reflect.TypeOf(spawnParams{})
	seen := 0
	for i := 0; i < rt.NumField(); i++ {
		tag := strings.Split(rt.Field(i).Tag.Get("json"), ",")[0]
		if tag == "" || tag == "-" {
			continue // unexported / deliberately off the wire
		}
		seen++
		if !set[tag] {
			t.Errorf("spawnParams.%s decodes %q, which internal/bus/spawnkeys.go does not list — "+
				"encoding/json binds %q to it case-insensitively, so the hub's spawn-authority gate "+
				"cannot see a variant spelling of this field. Add it to spawnParamKeys",
				rt.Field(i).Name, tag, strings.ToUpper(tag[:1])+tag[1:])
		}
	}
	if seen < 20 {
		t.Fatalf("only %d wire fields found on spawnParams — the reflection went blind, so this guard proved nothing", seen)
	}
}

// desktopSpawnDestructure pulls the identifier list out of hubCapabilities.ts's
// `registerCapability('agents.spawn', async (params: unknown) => { const { ...`
// destructure. A renamed binding (`provider: reqProvider`) is recorded under the
// WIRE name, which is the half before the colon — that is the name the bus sees.
var desktopSpawnDestructure = regexp.MustCompile(`(?s)registerCapability\('agents\.spawn'.*?const \{(.*?)\n\s*\} = \(params`)

// TestTheDesktopSpawnSurfaceIsInTheBusCanonicalKeySet is the other provider.
// It matters as much as the Go one: the hub does not know which provider will
// answer a spawn, so the list has to be the UNION, and a desktop-only field left
// out of it is a desktop-only bypass.
func TestTheDesktopSpawnSurfaceIsInTheBusCanonicalKeySet(t *testing.T) {
	set := canonicalSpawnKeySet(t)
	src := string(mustReadRepoFile(t, "apps", "desktop", "src", "main", "services", "hubCapabilities.ts"))
	m := desktopSpawnDestructure.FindStringSubmatch(src)
	if m == nil {
		t.Fatal("could not find the agents.spawn destructure in hubCapabilities.ts — this guard reads that block, " +
			"so a rename here turns it off silently rather than failing. Fix the pattern, do not delete the test")
	}
	var found []string
	for _, line := range strings.Split(m[1], "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "//") || strings.HasPrefix(line, "*") || strings.HasPrefix(line, "/*") {
			continue
		}
		name := strings.TrimSpace(strings.Split(strings.TrimSuffix(line, ","), ":")[0])
		if name == "" || strings.ContainsAny(name, " \t{}()") {
			continue
		}
		found = append(found, name)
	}
	sort.Strings(found)
	if len(found) < 20 {
		t.Fatalf("only %d fields parsed out of the desktop destructure (%v) — the parse went blind", len(found), found)
	}
	for _, name := range found {
		if !set[name] {
			t.Errorf("hubCapabilities.ts's agents.spawn reads %q, which internal/bus/spawnkeys.go does not list — "+
				"add it to spawnParamKeys or the hub cannot refuse a variant spelling of it", name)
		}
	}
}
