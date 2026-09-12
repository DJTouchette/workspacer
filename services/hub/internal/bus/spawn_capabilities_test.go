package bus

import "testing"

func TestHelloAdvertisesActualFullAccessAuthority(t *testing.T) {
	for _, tc := range []struct {
		name string
		conn *conn
		want bool
	}{
		{"host", &conn{trusted: true}, true},
		{"operator", &conn{trusted: true, viaScopedToken: true}, true},
		{"triage", &conn{scope: "triage", scopeMethods: []string{"agents.sendMessage"}, viaScopedToken: true}, false},
		{"plugin", &conn{pluginID: "p"}, false},
		{"peer without grant", &conn{trusted: true, federated: true}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := tc.conn.helloFrame().SpawnFullAccess; got != tc.want {
				t.Fatalf("full access = %v, want %v", got, tc.want)
			}
		})
	}
}
