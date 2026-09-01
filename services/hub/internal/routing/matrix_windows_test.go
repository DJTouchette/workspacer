//go:build windows

package routing

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestWindowsCeilingUsesVolumeCaseAndSeparatorSemantics(t *testing.T) {
	root := t.TempDir()
	locked := filepath.Join(root, "Locked")
	// routing.yaml is hand-written, so accept the alternate separator spelling
	// and case Windows resolves to the same directory.
	configured := swapASCIIPathCase(filepath.ToSlash(locked))
	m := &Matrix{Ceilings: map[string]Ceiling{
		CeilingDefaultKey: {MaxCapability: "frontier", MaxToolScope: "operator"},
		root:              {MaxCapability: "balanced", MaxToolScope: "triage"},
		configured:        {MaxCapability: "cheap", MaxToolScope: "view"},
	}}

	if c, key := m.CeilingFor(filepath.Join(locked, "Child")); key != configured || c.MaxToolScope != "view" {
		t.Fatalf("case/separator variant missed the nearest Windows ancestor: key=%q ceiling=%+v", key, c)
	}
	if _, key := m.CeilingFor(locked + "-sibling"); key != root {
		t.Errorf("prefix sibling matched %q, want the shallower real ancestor %q", key, root)
	}

	volume := filepath.VolumeName(root)
	if len(volume) == 2 && volume[1] == ':' {
		otherDrive := "Z:"
		if strings.EqualFold(volume, otherDrive) {
			otherDrive = "Y:"
		}
		if _, key := m.CeilingFor(otherDrive + locked[len(volume):]); key != CeilingDefaultKey {
			t.Errorf("a path on volume %s matched ceiling %q on volume %s", otherDrive, key, volume)
		}
	}
}

func swapASCIIPathCase(s string) string {
	b := []byte(s)
	for i, c := range b {
		switch {
		case c >= 'a' && c <= 'z':
			b[i] = c - ('a' - 'A')
		case c >= 'A' && c <= 'Z':
			b[i] = c + ('a' - 'A')
		}
	}
	return string(b)
}
