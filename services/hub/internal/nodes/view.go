package nodes

import "time"

// NodeView is what a bus caller is told about a node, and it is a SEPARATE
// STRUCT rather than a redaction of [Node].
//
// The direction matters and is the whole design. A "hide these fields"
// projection re-opens itself every time the record grows a field, and Node is
// a record that will grow — a region, a size, a second cloud, a health URL.
// This struct is built by naming what goes IN, so a new field on Node is
// absent from every client until somebody comes here and decides otherwise.
// (The argument is plugin.PublicManifest's, applied to a record whose secret
// spends money rather than one whose secret is an argv.)
//
// DELIBERATELY ABSENT, each with the reason it is absent:
//
//   - Fly.Token — the credential. It can start, stop and CREATE machines on
//     the app it is scoped to, which is to say it can spend money. It lives in
//     a 0600 file and has no business anywhere else. Pinned by
//     TestNodeViewNeverCarriesTheFlyToken.
//   - Fly.TokenFile — a path on the hub's disk that names where a secret is
//     kept. The plugin manifest's proven case was an `args` entry reading
//     ["--api-key-file","~/.ssh/id_ed25519"]: the LOCATION of a secret is
//     worth withholding on its own.
//   - Fly.App / Fly.MachineID — not credentials, but nothing a client does
//     needs them: every method here takes a node ID. Naming another party's
//     infrastructure to a phone token buys nothing and forecloses nothing.
//   - Fly.BaseURL — an endpoint, frequently the private-network one, which
//     names the org's internal API address.
//   - the exit record's own bootId and machine id — [ExitRecord] is itself a
//     projection of the file on the node's disk, for the same reason.
//
// What IS here is what a wake button has to render: which node, what it is
// called, what state it is in, when it entered that state, when its provider
// last answered, a sentence saying why, whether the hub can wake it at all,
// and how many wake attempts in a row have failed.
type NodeView struct {
	// ID is the handle every nodes.* method takes.
	ID string `json:"id"`
	// Label is the human name; always populated (it falls back to ID).
	Label string `json:"label"`
	// State is one of the four [State] values, as a string.
	State string `json:"state"`
	// Since is unix MILLISECONDS of when this node entered State. Zero when
	// unknown. Milliseconds, matching federation.PeerInfo.LastSeen — note
	// that the hub.peer.* EVENTS use RFC3339 instead, and the node events
	// here deliberately do not repeat that split: everything on this path is
	// unix milliseconds.
	Since int64 `json:"since,omitempty"`
	// LastSeen is unix milliseconds of the last time this node's provider
	// answered a liveness probe. Zero = never, in this hub's lifetime.
	LastSeen int64 `json:"lastSeen,omitempty"`
	// Detail is one sentence a person can act on. It never names a credential,
	// a token file or an endpoint.
	Detail string `json:"detail,omitempty"`
	// Wakeable reports whether the hub holds enough (coordinates AND a
	// credential) to start this node itself. A client should not offer a wake
	// button for a node that says false — it would fail every time.
	Wakeable bool `json:"wakeable"`
	// LastExit is the node's own account of how its PREVIOUS run ended, when
	// the hub has been able to read it. It is the only thing that can tell a
	// machine somebody put to sleep from one that crashed and gave up, because
	// the cloud API reports both as `stopped`. Absent when unknown — which is
	// most of the time, including for every node the hub has not yet seen up.
	LastExit *ExitRecord `json:"lastExit,omitempty"`
	// WakeFailures counts consecutive wake attempts that did not end with the
	// node's provider registering. It exists because a Fly machine that
	// crash-loops on boot ends up `stopped`, which is INDISTINGUISHABLE
	// through the cloud API from a healthy sleeping node. A nonzero count is
	// how a client tells "asleep" from "asleep because it keeps dying".
	WakeFailures int `json:"wakeFailures,omitempty"`
}

// ViewOf renders one node for a caller. Every field is assigned explicitly;
// there is no path from a Node to a NodeView that does not pass through here.
func ViewOf(n Node, s State, since, lastSeen time.Time, detail string, wakeFailures int) NodeView {
	v := NodeView{
		ID:           n.ID,
		Label:        n.DisplayName(),
		State:        string(s),
		Detail:       detail,
		Wakeable:     n.Wakeable(),
		WakeFailures: wakeFailures,
	}
	if !since.IsZero() {
		v.Since = since.UnixMilli()
	}
	if !lastSeen.IsZero() {
		v.LastSeen = lastSeen.UnixMilli()
	}
	return v
}
