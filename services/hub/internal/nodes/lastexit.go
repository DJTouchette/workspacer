package nodes

import "strings"

// ExitRecord is a node's own account of how its PREVIOUS run ended, as written
// by the node's entrypoint and relayed on brain.info.
//
// It exists because the cloud API cannot answer the question. Fly's
// `on-failure` restart policy retries a crashing machine and then leaves it
// `stopped` — identical, through the API, to a machine somebody put to sleep
// on purpose. Two opposite meanings, one state string.
//
// This is a PROJECTION, like [NodeView], and for the same reason: the record
// on disk also carries a bootId and the Fly machine id, and a client needs
// neither. What goes in is what a person reads.
type ExitRecord struct {
	// Reason is the entrypoint's own word for the ending: "signal-TERM" and
	// "signal-INT" are deliberate stops; "claudemon-died", "brain-died",
	// "boot-failure" and "unknown" are not.
	Reason string `json:"reason"`
	// ExitCode is the process tree's exit status, when the entrypoint recorded
	// one.
	ExitCode *int `json:"exitCode,omitempty"`
	// At is when the ending was recorded, in the entrypoint's own format
	// (RFC3339, UTC, second resolution). A STRING and not a millisecond
	// stamp, deliberately: it is the node's clock, not the hub's, and a
	// machine that has been stopped and started has unreliable opinions about
	// time. It is for a human to read, never for the hub to compute with.
	At string `json:"at,omitempty"`
}

// Clean reports whether this ending was a deliberate stop rather than a
// failure. Anything the hub does not recognise is NOT clean — the direction
// that errs toward telling the user to look.
func (e *ExitRecord) Clean() bool {
	if e == nil {
		return false
	}
	return strings.HasPrefix(e.Reason, "signal-")
}

// Describe renders the record as one sentence for a NodeView detail.
func (e *ExitRecord) Describe() string {
	if e == nil || e.Reason == "" {
		return ""
	}
	at := ""
	if e.At != "" {
		at = " at " + e.At
	}
	if e.Clean() {
		return "its previous run ended cleanly (" + e.Reason + ")" + at
	}
	return "ITS PREVIOUS RUN DID NOT END CLEANLY (" + e.Reason + ")" + at +
		" — the machine failed rather than being put to sleep"
}
