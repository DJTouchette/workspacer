package bus

import (
	"encoding/json"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"testing"
)

func TestAuthenticatedRoutingHostIdentity(t *testing.T) {
	for _, tc := range []struct {
		name, host, query string
		want              bool
	}{
		{"host", "host-secret", "?token=host-secret", true},
		{"untokened", "", "", false},
		{"operator", "host-secret", "?token=tok-operator", false},
		{"peer", "host-secret", "?token=host-secret&peer=1", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			srv := NewServer(broker.New())
			srv.SetToken(tc.host)
			installScopedTiers(srv)
			srv.RegisterLocalIdent("routing.preferences.get", func(c CallerIdentity, _ json.RawMessage) (any, error) { return c, nil })
			url := serve(t, srv)
			c := dialRaw(t, url, tc.query)
			readUntil(t, c, "hello")
			send(t, c, Frame{Op: "call", ID: "identity", Method: "routing.preferences.get"})
			r := readUntil(t, c, "result")
			var id CallerIdentity
			if e := json.Unmarshal(r.Result, &id); e != nil {
				t.Fatal(e)
			}
			if id.AuthenticatedHost != tc.want {
				t.Fatalf("wrong host identity %+v", id)
			}
		})
	}
}
