package main

import (
	"strings"
	"testing"
)

// The effort chip calls a distinct capability for Claude. Keep the gate and
// the per-chip disabled state load-bearing: checking only claude.setModel makes
// a scoped phone offer a control its token cannot invoke.
func TestMobileClaudeEffortChipRequiresSetEffort(t *testing.T) {
	source := string(mobileHTML)
	for _, required := range []string{
		"can(caps.provider === 'claude' ? 'claude.setEffort' : 'claude.setModel')",
		"b.disabled = b.dataset.chip === 'effort' ? !effortLive : !live;",
	} {
		if !strings.Contains(source, required) {
			t.Fatalf("mobile Claude effort capability gate is missing %q", required)
		}
	}
}
