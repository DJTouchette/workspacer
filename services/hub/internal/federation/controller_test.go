package federation

import (
	"context"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"testing"
	"time"
)

func TestControllerPreservesUnchangedLinks(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := Peer{Name: "a", URL: "ws://127.0.0.1:1/bus", Token: "credential"}
	controller, err := NewController(ctx, broker.New(), []Peer{a})
	if err != nil {
		t.Fatal(err)
	}
	original := controller.manager().Client("a")
	b := Peer{Name: "b", URL: "ws://127.0.0.1:2/bus"}
	if err := controller.Replace([]Peer{a, b}); err != nil {
		t.Fatal(err)
	}
	if controller.manager().Client("a") != original {
		t.Fatal("adding another peer disconnected the unchanged link")
	}
	a.Token = "rotated"
	if err := controller.Replace([]Peer{a}); err != nil {
		t.Fatal(err)
	}
	if controller.manager().Client("a") == original || controller.HasPeer("b") {
		t.Fatal("changed/removed links did not reload")
	}
}

func TestForwardingBudgetPreservesLongOperations(t *testing.T) {
	if forwardingBudget("agents.spawn") != 355*time.Second {
		t.Fatal("peer spawn lost its worktree/setup budget")
	}
	if forwardingBudget("claude.handoffAgentBrief") != 175*time.Second {
		t.Fatal("peer handoff lost its authored-brief budget")
	}
	if forwardingBudget("sessions.snapshots") != 25*time.Second {
		t.Fatal("ordinary peer reads were lengthened")
	}
}
