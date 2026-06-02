package event

import "testing"

func TestMatches(t *testing.T) {
	cases := []struct {
		pattern, typ string
		want         bool
	}{
		{"agent.spawned", "agent.spawned", true},
		{"agent.spawned", "agent.terminated", false},
		{"agent.*", "agent.spawned", true},
		{"agent.*", "agent.state.changed", true},
		{"agent.*", "agent", false},     // bare namespace is not under "agent."
		{"agent.*", "agentx.foo", false}, // prefix must respect the dot
		{"*", "anything.at.all", true},
		{"git.changed", "agent.spawned", false},
	}
	for _, c := range cases {
		if got := Matches(c.pattern, c.typ); got != c.want {
			t.Errorf("Matches(%q,%q)=%v want %v", c.pattern, c.typ, got, c.want)
		}
	}
}

func TestMatchesAny(t *testing.T) {
	patterns := []string{"git.changed", "agent.*"}
	if !MatchesAny(patterns, "agent.done") {
		t.Error("expected agent.done to match agent.*")
	}
	if MatchesAny(patterns, "session.usage") {
		t.Error("session.usage should not match")
	}
	if MatchesAny(nil, "agent.spawned") {
		t.Error("empty pattern set should match nothing")
	}
}
