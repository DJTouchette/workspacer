package main

import (
	"bytes"
	"context"
	"encoding/json"
	"log"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Omitted-skipPermissions default resolution, provider half. The desktop spawn
// dialog pre-selects the config default (claude.skipPermissionsDefault / a
// bypass defaultPermissionMode); a bus spawn that OMITS the field must resolve
// the same default — and the resolved value passes the SAME grant gate as an
// explicit request: honored only under the hub-stamped yoloGranted, clamped
// (and logged) for everyone else. Twin of the facade's spawndefaults_test.go;
// this half matters for direct bus callers (web, jobs, plugins), since the
// facade now always forwards an explicit value.

// writeSkipDefaultConfig writes a config.yaml into the test's isolated config
// home (newSpawnTestRegistry's tempConfigHome), so r.cfg resolves these values.
func writeSkipDefaultConfig(t *testing.T, yaml string) {
	t.Helper()
	dir := os.Getenv("XDG_CONFIG_HOME")
	if dir == "" {
		t.Fatal("expected tempConfigHome to have set XDG_CONFIG_HOME")
	}
	p := filepath.Join(dir, "workspacer", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(yaml), 0o644); err != nil {
		t.Fatal(err)
	}
}

// managedSpawnRecorder is the fake claudemon for the shipping default leg
// (claude.transport=stream → /sessions/spawn-managed).
func managedSpawnRecorder(t *testing.T, gotBody *spawnManagedReq) *httptest.Server {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": "s1"})
	}))
	t.Cleanup(srv.Close)
	return srv
}

// TestSpawnOmittedSkipResolvesConfigDefaultWhenGranted: granted caller +
// omitted field + claude.skipPermissionsDefault:true → the worker spawns
// bypassed, matching what the desktop dialog would do.
func TestSpawnOmittedSkipResolvesConfigDefaultWhenGranted(t *testing.T) {
	var gotBody spawnManagedReq
	srv := managedSpawnRecorder(t, &gotBody)
	reg := newSpawnTestRegistry(t, srv.URL)
	writeSkipDefaultConfig(t, "claude:\n  skipPermissionsDefault: true\n")

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","yoloGranted":true}`)); err != nil {
		t.Fatal(err)
	}
	if !gotBody.Yolo {
		t.Errorf("granted spawn with omitted skipPermissions must resolve the config default to yolo=true, got %+v", gotBody)
	}
}

// TestSpawnOmittedSkipHonorsABypassDefaultPermissionMode: the other config
// spelling — defaultPermissionMode: bypassPermissions with the toggle off.
func TestSpawnOmittedSkipHonorsABypassDefaultPermissionMode(t *testing.T) {
	var gotBody spawnManagedReq
	srv := managedSpawnRecorder(t, &gotBody)
	reg := newSpawnTestRegistry(t, srv.URL)
	writeSkipDefaultConfig(t, "claude:\n  skipPermissionsDefault: false\n  defaultPermissionMode: bypassPermissions\n")

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","yoloGranted":true}`)); err != nil {
		t.Fatal(err)
	}
	if !gotBody.Yolo {
		t.Errorf("a bypass defaultPermissionMode must resolve the omitted field to yolo=true, got %+v", gotBody)
	}
}

// TestSpawnConfigDefaultClampedAndLoggedWithoutTheGrant: the SECURITY half —
// the config default passes the same gate as an explicit request. An unstamped
// caller's defaulted bypass is clamped, and the strip is logged with its
// config-default provenance so the silently-approvals-on worker is diagnosable.
func TestSpawnConfigDefaultClampedAndLoggedWithoutTheGrant(t *testing.T) {
	var gotBody spawnManagedReq
	srv := managedSpawnRecorder(t, &gotBody)
	reg := newSpawnTestRegistry(t, srv.URL)
	writeSkipDefaultConfig(t, "claude:\n  skipPermissionsDefault: true\n")

	var buf bytes.Buffer
	prev := log.Writer()
	log.SetOutput(&buf)
	defer log.SetOutput(prev)

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp"}`)); err != nil {
		t.Fatal(err)
	}
	if gotBody.Yolo {
		t.Errorf("an unstamped caller's config-defaulted bypass must be clamped, got %+v", gotBody)
	}
	if out := buf.String(); !strings.Contains(out, "config default") || !strings.Contains(out, "full-access grant") {
		t.Errorf("clamped config default must be logged with its provenance, got:\n%s", out)
	}
}

// TestSpawnOmittedSkipWithDefaultOffStaysOff: shipped default (both config
// values off) + omitted field → approvals on, even for a granted caller, and
// no clamp line in the log (nothing was stripped).
func TestSpawnOmittedSkipWithDefaultOffStaysOff(t *testing.T) {
	var gotBody spawnManagedReq
	srv := managedSpawnRecorder(t, &gotBody)
	reg := newSpawnTestRegistry(t, srv.URL)

	var buf bytes.Buffer
	prev := log.Writer()
	log.SetOutput(&buf)
	defer log.SetOutput(prev)

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","yoloGranted":true}`)); err != nil {
		t.Fatal(err)
	}
	if gotBody.Yolo {
		t.Errorf("default off: omitted skipPermissions must stay false, got %+v", gotBody)
	}
	if strings.Contains(buf.String(), "full-access grant") {
		t.Errorf("nothing was stripped, so nothing should be logged, got:\n%s", buf.String())
	}
}

// TestSpawnExplicitFalseBeatsTheConfigDefault: an explicit caller value —
// including false — always wins over the config default.
func TestSpawnExplicitFalseBeatsTheConfigDefault(t *testing.T) {
	var gotBody spawnManagedReq
	srv := managedSpawnRecorder(t, &gotBody)
	reg := newSpawnTestRegistry(t, srv.URL)
	writeSkipDefaultConfig(t, "claude:\n  skipPermissionsDefault: true\n")

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","skipPermissions":false,"yoloGranted":true}`)); err != nil {
		t.Fatal(err)
	}
	if gotBody.Yolo {
		t.Errorf("explicit skipPermissions:false must beat the config default, got %+v", gotBody)
	}
}

// TestPermissionModeMeansBypassIsANarrowSet: the config-default resolver's
// vocabulary. Only the spellings known to mean bypass count; a garbled config
// value resolves to approvals ON — the opposite fail-closed direction from
// isPermissionEscalation, which judges REQUESTS and refuses unknowns.
func TestPermissionModeMeansBypassIsANarrowSet(t *testing.T) {
	for _, mode := range []string{"bypassPermissions", "yolo"} {
		if !permissionModeMeansBypass(mode) {
			t.Errorf("permissionModeMeansBypass(%q) = false — the config default would silently stop applying", mode)
		}
	}
	for _, mode := range []string{"", "default", "plan", "acceptEdits", "made-up-mode"} {
		if permissionModeMeansBypass(mode) {
			t.Errorf("permissionModeMeansBypass(%q) = true — a non-bypass (or garbled) config value must resolve to approvals ON", mode)
		}
	}
}
