// Package nodes is the hub's registry of REMOTE WORKER NODES: machines that
// are not this one, that run a capability provider which dials in here, and
// that may be intentionally switched off.
//
// The last clause is the whole reason the package exists. Everything else the
// hub knows about a target is binary — a federation peer is `connected` or it
// is not, a provider is registered or it is not — and a node that is asleep on
// purpose is neither of those things. Collapsing "asleep, and one button away"
// into "unreachable" is what makes a remote node feel broken to use, so this
// package's central contribution is a state a machine can be in on purpose:
// [StateWaking], and the [StateStopped] it comes from.
//
// The transport is NOT here and is not new. A node is claudemon plus
// `brain --hub wss://…/bus --token …`, which is an ordinary reconnecting
// capability PROVIDER: from the hub's point of view a woken node's sessions
// are local sessions. This package never dials a node, never holds a node's
// address, and adds nothing to the bus protocol. What it adds is (a) a record
// of which machines exist, (b) a state machine over their liveness, and (c)
// the one out-of-band call that can start a machine that is not running.
package nodes

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

// State is what the hub currently believes about a node. Four values, and the
// distinction between the last three is the point:
//
//   - available   — its provider is on the bus and answering.
//   - waking      — the hub has asked the cloud API to start it and is waiting
//     for the provider to register. Intentional, bounded, and the state a
//     client should render as "starting, ~20s" with a disabled composer.
//   - stopped     — it is switched off, deliberately, and can be woken.
//   - unreachable — the hub does not know how to get a working node out of
//     this, and a person should look. NOT the same as stopped.
type State string

const (
	StateAvailable   State = "available"
	StateWaking      State = "waking"
	StateStopped     State = "stopped"
	StateUnreachable State = "unreachable"
)

// Valid reports whether s is one of the four states.
func (s State) Valid() bool {
	switch s {
	case StateAvailable, StateWaking, StateStopped, StateUnreachable:
		return true
	}
	return false
}

// Fly is a node's Fly Machines API coordinates, plus the credential that lets
// the hub act on them.
//
// THE TOKEN LIVES HERE AND NOWHERE ELSE, and that placement is the security
// decision this file is really about. It is NOT in config.yaml, because
// config.yaml is credential-free by design and that is exactly what keeps
// `config.get` in the view tier — a token there would be readable by any
// phone that can render the UI. It is NOT a flag, because a flag puts it in
// argv and /proc/<pid>/cmdline is world-readable. It is in a 0600 file of its
// own in the config dir, which is precisely the reasoning peers.json already
// carries for peer bearer tokens (internal/federation.DefaultPeersPath).
//
// And it never leaves this struct: nothing client-facing is derived from a
// Node by redaction. The projection a caller sees is [NodeView], built by
// naming what goes IN.
type Fly struct {
	// App is the Fly app the machine belongs to. Scope the token to this app
	// and nothing wider: an app-scoped Fly token is full control of that app
	// including creating machines, so the blast radius should be one app.
	App string `json:"app"`
	// MachineID is the machine to start.
	MachineID string `json:"machineId"`
	// Token is the Fly API token, inline. Prefer TokenFile or the environment
	// if you would rather the credential not sit in the same file as the
	// topology.
	Token string `json:"token,omitempty"`
	// TokenFile is a path to a file whose entire (trimmed) contents are the
	// token — the shape a mounted secret takes.
	TokenFile string `json:"tokenFile,omitempty"`
	// BaseURL overrides the Machines API endpoint. Empty means Fly's public
	// one; the value worth setting is http://_api.internal:4280, which is the
	// same API reachable from INSIDE the org's private network.
	BaseURL string `json:"baseUrl,omitempty"`
}

// Node is one registry entry, as written in nodes.json.
type Node struct {
	// ID is the stable handle every bus method takes. Letters, digits, - and _.
	ID string `json:"id"`
	// Label is what a person calls it. Falls back to ID when empty.
	Label string `json:"label,omitempty"`
	// Fly is nil for a node the hub can observe but not wake — a machine
	// somebody else powers on. Such a node is never reported `stopped`,
	// because without the cloud API the hub genuinely cannot tell "off on
	// purpose" from "broken", and guessing is the failure this package exists
	// to prevent.
	Fly *Fly `json:"fly,omitempty"`
}

// Wakeable reports whether the hub holds enough to start this node itself.
func (n Node) Wakeable() bool {
	return n.Fly != nil && n.Fly.App != "" && n.Fly.MachineID != ""
}

// DisplayName is Label, or ID when there is no label.
func (n Node) DisplayName() string {
	if n.Label != "" {
		return n.Label
	}
	return n.ID
}

// DefaultPath is where the registry lives: <config>/workspacer/nodes.json,
// next to peers.json and tokens.json.
//
// A separate file for the same two reasons peers.json is one — it carries a
// BEARER CREDENTIAL for a remote system, and config.yaml is credential-free by
// design — plus a third that is specific to this one: the Fly token can start,
// stop and create machines, which is to say it can spend money. There is
// deliberately no flag form. A `-node` flag would put the token in argv.
func DefaultPath() string {
	return filepath.Join(authtoken.ConfigDir(), "nodes.json")
}

// FlyTokenEnv is the last place a token is looked for, after the entry's own
// `token` and `tokenFile`. It exists because a hub deployed ON Fly receives
// its secrets as environment variables and there is no file to point at.
const FlyTokenEnv = "FLY_API_TOKEN"

// LoadFile reads nodes.json: a JSON array of entries.
//
// A missing file is no nodes (the ordinary desktop install, where this whole
// subsystem stays dormant). A corrupt file is an ERROR, deliberately loud, for
// the same reason peers.json is: a typo that silently disables the registry
// reads to the user as "my remote machine vanished", which is the most
// expensive possible way to learn about a missing comma.
func LoadFile(path string) ([]Node, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var raw []Node
	if err := json.Unmarshal(b, &raw); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	seen := map[string]bool{}
	out := make([]Node, 0, len(raw))
	for _, n := range raw {
		n.ID = strings.TrimSpace(n.ID)
		n.Label = strings.TrimSpace(n.Label)
		if n.ID == "" {
			return nil, fmt.Errorf("%s: every node entry needs an id", path)
		}
		if !ValidID(n.ID) {
			return nil, fmt.Errorf("%s: node id %q: use letters, digits, - or _", path, n.ID)
		}
		if seen[n.ID] {
			return nil, fmt.Errorf("%s: duplicate node id %q", path, n.ID)
		}
		seen[n.ID] = true
		if n.Fly != nil {
			n.Fly.App = strings.TrimSpace(n.Fly.App)
			n.Fly.MachineID = strings.TrimSpace(n.Fly.MachineID)
			n.Fly.Token = strings.TrimSpace(n.Fly.Token)
			n.Fly.TokenFile = strings.TrimSpace(n.Fly.TokenFile)
			n.Fly.BaseURL = strings.TrimSpace(n.Fly.BaseURL)
			// Half a coordinate is a configuration mistake that would show up
			// as a node that silently never wakes. Refuse it here instead.
			if (n.Fly.App == "") != (n.Fly.MachineID == "") {
				return nil, fmt.Errorf("%s: node %q: fly needs BOTH app and machineId (or neither)", path, n.ID)
			}
		}
		out = append(out, n)
	}
	return out, nil
}

// ResolveToken finds the Fly token for a node: the entry's own `token`, then
// the contents of `tokenFile`, then $FLY_API_TOKEN. Empty means the hub holds
// no credential for this node, which makes it observable but not wakeable.
//
// Never a flag, in any of the three. See [DefaultPath].
func ResolveToken(f *Fly) (string, error) {
	if f == nil {
		return "", nil
	}
	if f.Token != "" {
		return f.Token, nil
	}
	if f.TokenFile != "" {
		b, err := os.ReadFile(f.TokenFile)
		if err != nil {
			return "", fmt.Errorf("fly tokenFile: %w", err)
		}
		return strings.TrimSpace(string(b)), nil
	}
	return strings.TrimSpace(os.Getenv(FlyTokenEnv)), nil
}

// ValidID constrains a node id to characters that are safe in a log line, a
// URL path segment and a JSON key alike.
func ValidID(s string) bool {
	if s == "" || len(s) > 64 {
		return false
	}
	for _, r := range s {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9', r == '-', r == '_':
		default:
			return false
		}
	}
	return true
}

// Who can read this file is answered by [FileExposure], in exposure.go — a
// THREE-valued answer, because the obvious one-liner
// (os.Stat(path).Mode().Perm()&0o077 != 0) is a constant on Windows and said
// "exposed" about every file that existed. See exposure.go.
