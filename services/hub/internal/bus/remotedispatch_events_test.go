package bus

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// Exercise the authenticated socket boundary: an operator is trusted for
// ordinary calls, but cannot mint a dispatch record or publish a worker result.
func TestDispatchEventsRefuseOperatorForgery(t *testing.T) {
	url, _ := scopedServer(t)
	// dialClientToken builds a query string; the second spelling authenticates
	// the same real token with the production federation downgrade flag.
	for _, token := range []string{"tok-operator", "tok-operator&peer=1", "host-secret&peer=1"} {
		operator := dialClientToken(t, url, token)
		for _, topic := range []string{TopicDispatchOpened, TopicDispatchRegistered, TopicDispatchFailed, TopicDispatchUpdate} {
			operator.send(Frame{Op: "publish", Event: &event.Envelope{
				Type: topic, Data: json.RawMessage(`{"ownerSessionId":"victim-manager","dispatchId":"0123456789abcdef"}`),
			}})
			if got := operator.readUntil("error"); !strings.Contains(got.Error, "authorized") {
				t.Fatalf("%s: expected authorization refusal, got %q", topic, got.Error)
			}
		}
	}
}

func TestDispatchHubStampCannotBePublished(t *testing.T) {
	url, _ := scopedServer(t)
	host := dialClientToken(t, url, "host-secret")
	host.send(Frame{Op: "publish", Event: &event.Envelope{
		Type: TopicDispatchUpdate, Hub: "guessed-peer",
		Data: json.RawMessage(`{"dispatchId":"0123456789abcdef"}`),
	}})
	if got := host.readUntil("error"); !strings.Contains(got.Error, "hub identity") {
		t.Fatalf("forged hub stamp was not rejected: %q", got.Error)
	}
}

func TestDispatchPublicationRequiresLocalExecutionAuthority(t *testing.T) {
	cases := []struct {
		name   string
		caller *conn
		want   bool
	}{
		{"host", &conn{trusted: true, authenticatedHost: true}, true},
		{"anonymous trusted", &conn{trusted: true}, false},
		{"paired operator", &conn{trusted: true, viaScopedToken: true}, false},
		{"federated host", &conn{trusted: true, federated: true}, false},
		{"spawn provider", &conn{scope: providerScope, scopeMethods: []string{}, provides: []string{spawnMethod}}, true},
		{"catalog provider", &conn{scope: providerScope, scopeMethods: []string{}, provides: []string{"config.get"}}, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := tc.caller.mayPublish(TopicDispatchUpdate); got != tc.want {
				t.Fatalf("dispatch update publish = %v, want %v", got, tc.want)
			}
			if tc.caller.mayPublish(TopicDispatchOpened) {
				t.Fatal("a socket credential can manufacture router admission")
			}
		})
	}
}

func TestDispatchReplayReplacesClaimedOriginWithCredentialIdentity(t *testing.T) {
	rt := &router{}
	raw, err := rt.sanitizeCallParams(&conn{tokenID: "actual-origin"}, "agents.dispatchReplay", json.RawMessage(`{"dispatchId":"0123456789abcdef","originKey":"forged-origin","ackedSeq":7}`))
	if err != nil {
		t.Fatal(err)
	}
	var params map[string]any
	if err := json.Unmarshal(raw, &params); err != nil {
		t.Fatal(err)
	}
	if params["originKey"] != "actual-origin" || params["ackedSeq"] != float64(7) {
		t.Fatalf("incorrect replay identity: %s", raw)
	}
}
