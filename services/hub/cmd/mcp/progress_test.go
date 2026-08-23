package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// TestReportProgressIsHeldByEveryTier: a VIEW worker holding this tool is the
// whole point — the alternative was dispatching a read-only scout at triage just
// so it could say "the approach you gave me is wrong", which also hands it
// approve/interrupt over other sessions.
func TestReportProgressIsHeldByEveryTier(t *testing.T) {
	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator} {
		if !listToolsFor(t, scope)["report_progress"] {
			t.Errorf("%s tier is missing report_progress", scope)
		}
	}
}

// …and it gets there the ORDINARY way, which is the property worth pinning
// rather than the membership above. report_progress was once the single tool
// registered without asking b.allowed — a hand-written exception sitting beside
// the tier derivation, which meant tiers_test.go's "the tools are what the scope
// may call" rule quietly had one member it did not explain. Now
// agents.reportProgress is in authtoken's viewMethods and the tool follows from
// it, so the two agree per tier: admit or withdraw the method and the tool moves
// with it, in one place.
//
// The trade this makes is real and is contained elsewhere: a scoped view/triage
// token may now CALL agents.reportProgress on the bus, where there is no
// credential to derive a sender from. That is harmless only because the router
// deletes `callerSessionId` from every untrusted caller — on the local path and
// the federated one — so such a call reaches the provider unattributed and is
// refused. Those two strips are pinned in internal/bus/reportprogress_test.go;
// if either goes, this derivation is no longer safe to keep.
func TestReportProgressToolFollowsTheTierAllowlist(t *testing.T) {
	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator} {
		mayCall := event.MatchesAny(scope.Methods(), reportProgressMethod)
		hasTool := listToolsFor(t, scope)["report_progress"]
		if mayCall != hasTool {
			t.Errorf("%s tier: may call %s = %v but holds the report_progress tool = %v — the tool must derive from the allowlist, not from an exception beside it",
				scope, reportProgressMethod, mayCall, hasTool)
		}
	}
}

// The containment is that the model cannot name either end of this channel. A
// session id in the input schema is a session id the model can set, so its
// ABSENCE is the invariant worth pinning — not the handler's behaviour with one.
func TestReportProgressSchemaExposesNoSessionID(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cs := connectToolClient(t, ctx, func(b *build) { addProgressTool(b) })
	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("ListTools: %v", err)
	}
	for _, tl := range tools.Tools {
		if tl.Name != "report_progress" {
			continue
		}
		raw, err := json.Marshal(tl.InputSchema)
		if err != nil {
			t.Fatalf("marshal schema: %v", err)
		}
		if strings.Contains(strings.ToLower(string(raw)), "session") {
			t.Fatalf("report_progress's input schema mentions a session: %s", raw)
		}
		return
	}
	t.Fatal("report_progress was not registered")
}

// The stamp itself: whatever the caller sends, `callerSessionId` on the wire is
// the one the FACADE resolved from the request's token record.
func TestReportProgressStampsTheCallerFromItsToken(t *testing.T) {
	cases := []struct {
		name, label, want string
	}{
		{"a per-session facade token", "session:abc-123", "abc-123"},
		{"the static MCP token", "", ""},
		{"a token whose label is not a session", "plugin:shiplight", ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			if c.label != "" {
				ctx = context.WithValue(ctx, tokenLabelKey{}, c.label)
			}

			var got map[string]any
			cs := connectToolClient(t, ctx, func(b *build) {
				b.caller = func(_ context.Context, _ string, params any) (json.RawMessage, error) {
					raw, err := json.Marshal(params)
					if err != nil {
						return nil, err
					}
					if err := json.Unmarshal(raw, &got); err != nil {
						return nil, err
					}
					return json.RawMessage(`{"deliveredTo":"mgr-1"}`), nil
				}
				addProgressTool(b)
			})

			res, err := cs.CallTool(ctx, &mcp.CallToolParams{
				Name:      "report_progress",
				Arguments: map[string]any{"note": "phase 1 landed", "needsDecision": true},
			})
			if err != nil {
				t.Fatalf("CallTool: %v", err)
			}
			if res.IsError {
				t.Fatalf("unexpected tool error: %s", resultText(res))
			}
			if got["callerSessionId"] != any(c.want) && !(c.want == "" && got["callerSessionId"] == nil) {
				t.Errorf("callerSessionId = %v, want %q", got["callerSessionId"], c.want)
			}
			if got["note"] != "phase 1 landed" || got["needsDecision"] != true {
				t.Errorf("the worker's own fields did not ride through: %v", got)
			}
		})
	}
}

// An empty identity is FORWARDED, not guessed at and not refused here: the
// provider owns the sentence explaining what a caller with no session should do
// instead, so there is one such sentence rather than two that can drift.
func TestReportProgressForwardsTheProvidersRefusal(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cs := connectToolClient(t, ctx, func(b *build) {
		b.caller = func(_ context.Context, _ string, _ any) (json.RawMessage, error) {
			return nil, errNoIdentity{}
		}
		addProgressTool(b)
	})
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "report_progress",
		Arguments: map[string]any{"note": "phase 1 landed"},
	})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	if !res.IsError {
		t.Fatal("a refused report must reach the model as a tool error, or a worker believes it reported when it did not")
	}
	if !strings.Contains(resultText(res), "could not identify your session") {
		t.Fatalf("the provider's own reason was not passed through: %s", resultText(res))
	}
}

type errNoIdentity struct{}

func (errNoIdentity) Error() string {
	return "report_progress: the host could not identify your session from your credential"
}

// connectToolClient builds an operator-tier server, lets the caller register
// tools on it, and returns a connected in-memory MCP client session.
func connectToolClient(t *testing.T, ctx context.Context, register func(*build)) *mcp.ClientSession {
	t.Helper()
	srv := mcp.NewServer(&mcp.Implementation{Name: "workspacer-test", Version: "v1"}, nil)
	b := &build{s: srv, scope: authtoken.ScopeOperator, allow: authtoken.ScopeOperator.Methods()}
	register(b)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := srv.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	t.Cleanup(func() { _ = cs.Close() })
	return cs
}
