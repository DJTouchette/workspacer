package nodes

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
	"time"
)

// The literal every test in this file hunts for. If it appears in anything a
// client can reach, the Fly credential has escaped the 0600 file it lives in.
const secretToken = "FlyV1_fm2_SECRET_THAT_MUST_NEVER_REACH_A_CLIENT"

func nodeWithSecret() Node {
	return Node{
		ID:    "fly-den",
		Label: "Fly node (den)",
		Fly: &Fly{
			App:       "wks-node-den",
			MachineID: "17811944b12345",
			Token:     secretToken,
			BaseURL:   "https://api.machines.dev",
		},
	}
}

// THE TOKEN-CONFINEMENT TEST.
//
// Everything a bus caller receives about a node is a NodeView. The view tier
// is a phone token. So the question this test asks is the only one that
// matters about where the credential lives: can a phone get it?
//
// The answer has to hold as the Node struct GROWS. That is why the check is
// not "does the view redact .Token" — a redaction list re-opens itself every
// time the record gains a field — but "does the rendered view contain any
// value from the record's credential-bearing half at all".
func TestNodeViewNeverCarriesTheFlyToken(t *testing.T) {
	v := ViewOf(nodeWithSecret(), StateStopped, time.Time{}, time.Time{}, "", 0)
	raw, err := json.Marshal(v)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if strings.Contains(string(raw), secretToken) {
		t.Fatalf("THE FLY TOKEN IS IN THE CLIENT-FACING VIEW: %s", raw)
	}
}

// The Fly coordinates are not a credential, but they are not needed by any
// client either — the buttons take a node id — and an allowlist projection
// means naming what goes IN. Anything a client does not need should not be on
// the wire where a later reader might assume it was considered.
func TestNodeViewDisclosesNoFlyIdentifiers(t *testing.T) {
	v := ViewOf(nodeWithSecret(), StateStopped, time.Time{}, time.Time{}, "", 0)
	raw, _ := json.Marshal(v)
	for _, leak := range []string{"wks-node-den", "17811944b12345", "api.machines.dev"} {
		if strings.Contains(string(raw), leak) {
			t.Errorf("NodeView discloses the Fly identifier %q: %s", leak, raw)
		}
	}
}

// THE PROJECTION MUST BE A SEPARATE STRUCT, not the record with fields hidden.
//
// If NodeView ever embeds Node (or otherwise reaches it), a field added to
// Node lands on the wire without anyone deciding it should. This is the same
// argument plugin.PublicManifest carries in its header, applied to a record
// that carries a credential rather than an argv.
func TestNodeViewIsAnAllowlistProjectionNotTheRecord(t *testing.T) {
	vt := reflect.TypeOf(NodeView{})
	for i := 0; i < vt.NumField(); i++ {
		f := vt.Field(i)
		if f.Anonymous {
			t.Fatalf("NodeView embeds %s — a projection must NAME every field it carries, "+
				"or the next field added to that struct ships to every client for free", f.Type)
		}
		if f.Type == reflect.TypeOf(Node{}) || f.Type == reflect.TypeOf(&Fly{}) || f.Type == reflect.TypeOf(Fly{}) {
			t.Fatalf("NodeView carries the record itself in field %s", f.Name)
		}
	}
}

// A node with no Fly credentials at all still renders — and says so, because
// "the hub cannot wake this" is exactly what a wake button needs to know
// before it offers itself.
func TestNodeViewSaysWhetherTheHubCanWakeIt(t *testing.T) {
	with := ViewOf(nodeWithSecret(), StateStopped, time.Time{}, time.Time{}, "", 0)
	if !with.Wakeable {
		t.Error("a node with fly coordinates should be wakeable")
	}
	without := ViewOf(Node{ID: "laptop"}, StateUnreachable, time.Time{}, time.Time{}, "", 0)
	if without.Wakeable {
		t.Error("a node with no fly coordinates must not advertise itself as wakeable")
	}
}
