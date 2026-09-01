//go:build windows

package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A Fleet Manager is itself the one live workspace root; no worker cwd is
// registered here. Its projects inherit that root by containment. This pins the
// exact manager call path that failed when callers used a lower-cased drive and
// slash-separated project path on Windows.
func TestBriefToolsAcceptWindowsManagerWorkspaceSpellings(t *testing.T) {
	sandbox := t.TempDir()
	setHome(t, filepath.Join(sandbox, "home"))
	setConfigHome(t, filepath.Join(sandbox, "config"))
	managerCwd := filepath.Join(sandbox, "FleetManager")
	project := filepath.Join(managerCwd, "Projects", "Client")
	outside := filepath.Join(sandbox, "Outside")
	for _, dir := range []string{managerCwd, project, outside} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	reg := registryWithCwd(t, managerCwd)
	projectVariant := windowsManagerPathVariant(project)
	call := func(method string, params map[string]any) error {
		body, err := json.Marshal(params)
		if err != nil {
			t.Fatal(err)
		}
		_, err = reg.handle(context.Background(), method, json.RawMessage(body))
		return err
	}

	if err := call("brief.append", map[string]any{"project": projectVariant, "section": "Recently", "line": "manager update"}); err != nil {
		t.Fatalf("brief.append rejected a project under the manager root with equivalent Windows spelling: %v", err)
	}
	if err := call("brief.check", map[string]any{"project": projectVariant}); err != nil {
		t.Fatalf("brief.check rejected a project under the manager root with equivalent Windows spelling: %v", err)
	}
	if err := call("brief.archive", map[string]any{"project": projectVariant, "section": "Recently", "count": 1, "keep": 0}); err != nil {
		t.Fatalf("brief.archive rejected a project under the manager root with equivalent Windows spelling: %v", err)
	}

	outsideVariant := windowsManagerPathVariant(outside)
	for _, tc := range []struct {
		method string
		params map[string]any
	}{
		{"brief.append", map[string]any{"project": outsideVariant, "section": "Recently", "line": "must not write"}},
		{"brief.check", map[string]any{"project": outsideVariant}},
		{"brief.archive", map[string]any{"project": outsideVariant, "section": "Recently", "count": 1, "keep": 0}},
	} {
		if err := call(tc.method, tc.params); err == nil || !strings.Contains(err.Error(), refusalText) {
			t.Errorf("%s accepted an outside project through the manager root: %v", tc.method, err)
		}
	}
}

func windowsManagerPathVariant(p string) string {
	p = strings.ReplaceAll(p, `\`, "/")
	var b strings.Builder
	b.Grow(len(p))
	for i := 0; i < len(p); i++ {
		c := p[i]
		if c >= 'a' && c <= 'z' {
			b.WriteByte(c - ('a' - 'A'))
		} else if c >= 'A' && c <= 'Z' {
			b.WriteByte(c + ('a' - 'A'))
		} else {
			b.WriteByte(c)
		}
	}
	return b.String()
}
