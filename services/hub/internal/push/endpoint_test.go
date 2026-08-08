package push

import (
	"encoding/json"
	"strings"
	"testing"
)

// PROVEN. push.subscribe (TRIAGE tier — the /m phone) made the hub POST to any
// URL the caller named. The endpoint is stored by one call and used by a
// different subsystem (Watch -> onSnapshot -> sendOne) to issue a request from
// the HOST's network position, for a tier that holds no fetch, no exec, no fs
// and no config capability at all — and the same tier holds agents.sendMessage
// and claude.approve, so it can pull the trigger on demand.
func TestPushSubscribeRefusesNonPushEndpoints(t *testing.T) {
	m, _ := newTestManager(t)
	who := Subscriber{TokenID: "phone-token", Scope: "triage"}

	refused := []string{
		"http://127.0.0.1:7895/plugins/install",
		"http://169.254.169.254/latest/meta-data/",
		"https://169.254.169.254/latest/meta-data/",
		"https://10.0.0.5/internal/admin",
		"https://192.168.1.1/",
		"https://[::1]/x",
		"https://[::ffff:127.0.0.1]/x",
		"https://[fd00::1]/x",
		"file:///etc/passwd",
		"ftp://example.invalid/x",
		"not a url at all",
	}
	for _, ep := range refused {
		params, _ := json.Marshal(map[string]any{
			"endpoint": ep,
			"keys":     map[string]string{"p256dh": "k", "auth": "a"},
		})
		if _, err := m.RPCSubscribeAs(who, params); err == nil {
			t.Errorf("push.subscribe accepted %q — the hub would POST there with a VAPID header from its own network position", ep)
		}
	}
	if n := len(m.subs); n != 0 {
		t.Fatalf("%d refused endpoints were stored anyway", n)
	}

	// FLOOR: a real Web Push endpoint must still work, or the fix is a deletion
	// of the feature. This is the shape every browser produces.
	for _, ep := range []string{
		"https://fcm.googleapis.com/fcm/send/abc123",
		"https://updates.push.services.mozilla.com/wpush/v2/gAAAA",
		"https://push.example.com:8443/p/xyz",
	} {
		params, _ := json.Marshal(map[string]any{
			"endpoint": ep,
			"keys":     map[string]string{"p256dh": "k", "auth": "a"},
		})
		if _, err := m.RPCSubscribeAs(who, params); err != nil {
			t.Errorf("push.subscribe refused a legitimate endpoint %q: %v", ep, err)
		}
	}
	if n := len(m.subs); n != 3 {
		t.Fatalf("stored %d legitimate subscriptions, want 3", n)
	}
}

// The failure has to say what is wrong, or the phone's registration silently
// stops working and nobody can tell why.
func TestPushEndpointRefusalNamesTheReason(t *testing.T) {
	err := validatePushEndpoint("http://127.0.0.1:9/x")
	if err == nil {
		t.Fatal("http loopback accepted")
	}
	if !strings.Contains(err.Error(), "https") {
		t.Errorf("error %q does not say what the endpoint must be", err)
	}
}
