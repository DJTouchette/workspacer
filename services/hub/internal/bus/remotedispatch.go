package bus

// REMOTE WORKER DISPATCH — the ORIGIN half: stamping per-dispatch provenance on
// an `agents.spawn` that is leaving this machine for a linked peer.
//
// THE PROBLEM. A Fleet Manager's whole doctrine is NEVER POLL: it dispatches,
// ends its turn, and is woken when the work comes home. Every wake in this
// codebase is PARENT-KEYED and LOCAL — the desktop's supervisorNudge and the
// brain's finishWatcher both require the worker's `parentSessionId` to name a
// session that is live, local and a wake target. So a manager here that spawned
// a worker on a peer named itself as the parent of a session on a machine where
// it does not exist: the peer's own wake router looked the id up, found nothing,
// and dropped the report. The dispatch worked and the answer went nowhere.
//
// THE FIX, and why it lives at this exact line. `federatedCall` is the ONE
// chokepoint every `hub:<peer>/agents.spawn` in the product passes through — the
// MCP facade's spawn_agent(hub:…), the desktop spawn dialog's Machine picker,
// /app, /m, wks-tui. It is also the only place that knows, with the router's own
// verified view of the caller, three things at once:
//
//   1. WHICH PEER the call is going to (so the answer's provenance can be bound
//      to that link, rather than to whatever a payload later claims);
//   2. that the operator has explicitly enabled that peer as a worker execution
//      target (federation.Peer.Dispatch) — being linked is not consent to open a
//      channel that injects turns into this machine's manager conversations;
//   3. that the caller is the local host control plane asserting a session
//      identity it was allowed to keep (`dispatchOwnerSessionId`, which
//      sanitizeSpawnParams deletes from every scoped, plugin or federated
//      caller).
//
// So the router mints an unguessable dispatch id, stamps it into the outbound
// params as `remoteOrigin`, and publishes the record locally BEFORE forwarding.
// The peer echoes that id — and nothing else about us — on every progress,
// blocked and finished callback it publishes onto its own bus, which arrives
// back over this same federation link stamped with the peer's name. The desktop
// then matches (link name + dispatch id) against its own record to find the
// manager. A peer never names a recipient; it acknowledges an id we gave it.
//
// WHAT THIS IS NOT. It is not a permission. `remoteOrigin` carries no grant, no
// token and no local path; the peer's authority is still exactly the link
// token's, clamped by sanitizeSpawnParams on both hops. And it is additive: a
// spawn to a peer that is NOT dispatch-enabled, or one with no owner session,
// forwards byte-for-byte as it did before — fire-and-forget, no return channel.

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"regexp"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// Topics published locally by the origin when a provenance-stamped dispatch is
// opened, confirmed, or fails to start. They are `agent.*` so an origin that is
// itself someone's peer forwards them under the ordinary allowlist rather than
// needing a new one — they disclose a peer name, a session id and a label, all
// of which that tier already sees on the fleet feed.
const (
	TopicDispatchOpened     = "agent.dispatch.opened"
	TopicDispatchRegistered = "agent.dispatch.registered"
	TopicDispatchFailed     = "agent.dispatch.failed"
	// TopicDispatchUpdate is the PEER-published callback this whole mechanism
	// exists to make routable. The origin never publishes it; it only ever
	// consumes one that arrived hub-stamped. Named here so both halves of the
	// contract are readable in one file.
	TopicDispatchUpdate = "agent.dispatch.update"
)

func isDispatchTopic(topic string) bool {
	switch topic {
	case TopicDispatchOpened, TopicDispatchRegistered, TopicDispatchFailed, TopicDispatchUpdate:
		return true
	default:
		return false
	}
}

// remoteOriginKey is the spawn param the stamp lands on. It is in
// spawnkeys.go's canonical set, so an aliased spelling is refused rather than
// smuggled past the delete below.
const remoteOriginKey = "remoteOrigin"

// dispatchOwnerKey is the host-derived calling-session stamp the MCP facade
// puts on a spawn (services/hub/cmd/mcp/main.go). It is the manager whose
// conversation a wake would be injected into, which is why a dispatch is opened
// only when it survived sanitizeSpawnParams.
const dispatchOwnerKey = "dispatchOwnerSessionId"

// DispatchIDPattern is the shape of a dispatch id, pinned so both the peer
// (which records and echoes it) and the origin (which looks it up) refuse a
// value that could be a path, an id-with-separator, or an unbounded blob. 32
// bytes of crypto/rand as hex is what mintDispatchID produces; the range is
// wider only so a future encoding is not a wire break.
var DispatchIDPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{16,128}$`)

// ValidDispatchID reports whether an id is well-formed. Exported because the
// consuming side (cmd/brain) applies the identical test to a value that arrived
// over the wire, and one regexp shared beats two that drift.
func ValidDispatchID(s string) bool { return DispatchIDPattern.MatchString(s) }

// mintDispatchID returns 64 hex chars of crypto/rand — an id that cannot be
// guessed by a peer-local observer and therefore cannot be used to address a
// callback at a dispatch it was not told about. Returns "" if the system RNG
// fails, which the caller treats as "do not open a dispatch" rather than
// falling back to something weaker.
func mintDispatchID() string {
	var b [32]byte
	if _, err := rand.Read(b[:]); err != nil {
		return ""
	}
	return hex.EncodeToString(b[:])
}

// dispatchStamp is what the origin adds to the outbound spawn params. Protocol
// is a plain integer rather than a semver: the peer refuses a number it does not
// know instead of guessing at a shape, which is how a NEWER origin talking to an
// OLDER peer fails loudly at spawn time rather than producing a worker whose
// answer silently has nowhere to go.
type dispatchStamp struct {
	Protocol   int    `json:"protocol"`
	DispatchID string `json:"dispatchId"`
}

// DispatchProtocol is the version this build stamps and accepts.
const DispatchProtocol = 1

// stampRemoteOrigin decides whether an outbound federated agents.spawn should
// carry dispatch provenance and, if so, rewrites its params.
//
// Returns the opened-record fields for the caller to publish, or ok=false when
// nothing should change. Every "no" here is a silent, byte-for-byte passthrough
// of the pre-existing behaviour — this function never refuses a spawn.
func stampRemoteOrigin(peer string, raw json.RawMessage, dispatchEnabled bool) (out json.RawMessage, rec map[string]any, ok bool) {
	if !dispatchEnabled || len(raw) == 0 {
		return raw, nil, false
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(raw, &m); err != nil || m == nil {
		return raw, nil, false
	}
	// Never inherit a caller-supplied one. sanitizeSpawnParams has already
	// deleted it for every non-federated caller and this path is by definition
	// not federated (an event that arrived over a link is not re-forwarded), so
	// this is belt: the stamp must be THIS router's, or absent.
	delete(m, remoteOriginKey)
	owner := stringField(m, dispatchOwnerKey)
	if owner == "" {
		// No manager to wake. A human clicking Spawn with the Machine picker
		// set, a plugin, a scoped token: all correct, and all unchanged.
		return raw, nil, false
	}
	id := mintDispatchID()
	if id == "" {
		return raw, nil, false
	}
	stamp, err := json.Marshal(dispatchStamp{Protocol: DispatchProtocol, DispatchID: id})
	if err != nil {
		return raw, nil, false
	}
	m[remoteOriginKey] = stamp
	next, err := json.Marshal(m)
	if err != nil {
		return raw, nil, false
	}
	return next, map[string]any{
		"dispatchId":     id,
		"protocol":       DispatchProtocol,
		"peer":           peer,
		"ownerSessionId": owner,
		// Descriptive only — what the local record shows an operator about a
		// dispatch whose worker card lives on another machine. None of it is
		// authority and none of it is a local path.
		"label":     stringField(m, "label"),
		"cwd":       stringField(m, "cwd"),
		"provider":  stringField(m, "provider"),
		"model":     stringField(m, "model"),
		"toolScope": stringField(m, "toolScope"),
	}, true
}

// stringField reads one string param, tolerating absence and a non-string
// spelling (which reads as empty rather than erroring — the provider's own
// decoder owns that complaint).
func stringField(m map[string]json.RawMessage, key string) string {
	r, ok := m[key]
	if !ok {
		return ""
	}
	var v string
	if json.Unmarshal(r, &v) != nil {
		return ""
	}
	return v
}

// spawnResultSessionID digs the new session's id out of a spawn result so the
// origin's record can be joined to the peer's card. Absent is fine: the record
// is already open under its dispatch id, which is what routing keys on.
func spawnResultSessionID(res json.RawMessage) string {
	if len(res) == 0 {
		return ""
	}
	var m map[string]json.RawMessage
	if json.Unmarshal(res, &m) != nil {
		return ""
	}
	return stringField(m, "sessionId")
}

// publishDispatchEvent is the router's nil-safe publish helper.
func (rt *router) publishDispatchEvent(topic string, data map[string]any) {
	rt.mu.Lock()
	pub := rt.publish
	rt.mu.Unlock()
	if pub == nil {
		return
	}
	pub(event.New(topic, "federation", data))
}
