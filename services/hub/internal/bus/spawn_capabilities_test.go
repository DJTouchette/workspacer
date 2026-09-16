package bus

import "testing"

func TestHelloAdvertisesProviderPermissionPassThroughToSpawnCapableCallers(t *testing.T) {
	for _, tc := range []struct {
		name string
		conn *conn
		want bool
	}{
		{"host", &conn{trusted: true}, true},
		{"operator", &conn{trusted: true, viaScopedToken: true}, true},
		{"triage", &conn{scope: "triage", scopeMethods: []string{"agents.sendMessage"}, viaScopedToken: true}, false},
		{"enabled plugin", &conn{pluginID: "p"}, true},
		{"peer without legacy grant", &conn{trusted: true, federated: true}, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := tc.conn.helloFrame().SpawnFullAccess; got != tc.want {
				t.Fatalf("full access = %v, want %v", got, tc.want)
			}
		})
	}
}
