package bus

import (
	"testing"
	"time"
)

func TestInteractionClockIgnoresReadsButNeverMutations(t *testing.T) {
	start := time.Unix(1000, 0)
	cn := &conn{}
	cn.markActive(start)
	cn.markInteraction(start)
	cn.reportsInteraction.Store(true)
	for _, method := range []string{"sessions.snapshots", "machine.power", "usage.report", "federation.peers", "config.get"} {
		cn.markCall(start.Add(time.Hour), method)
		if !cn.idleActivity().Equal(start) {
			t.Fatalf("poll %s moved the input clock", method)
		}
	}
	for _, method := range []string{"agents.sendMessage", "agents.spawn", "config.save", "sessions.terminalInput", "unknown.method"} {
		cn.markCall(start.Add(time.Hour), method)
		if !cn.idleActivity().Equal(start.Add(time.Hour)) {
			t.Fatalf("command %s did not count", method)
		}
		cn.markInteraction(start)
	}
	cn.markInteraction(start.Add(2 * time.Hour))
	if !cn.idleActivity().Equal(start.Add(2 * time.Hour)) {
		t.Fatal("real input was ignored")
	}
}

func TestLegacyClientsKeepTheirConservativeActivityClock(t *testing.T) {
	cn := &conn{}
	now := time.Now().Truncate(time.Millisecond)
	cn.markCall(now, "sessions.snapshots")
	if !cn.idleActivity().Equal(now) {
		t.Fatal("legacy client silently treated as idle")
	}
}
