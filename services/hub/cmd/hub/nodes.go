package main

// The hub-side wiring for the REMOTE NODE registry: which machines exist,
// which of them is awake, and the one out-of-band call that can start one that
// is not running.
//
// It lives in the hub, not in the brain and not in the desktop, for the same
// reason fleet.quiescence does: the hub is the process that exists in every
// deployment, and it is the only one still running when the node is off. A
// registry kept by the node could not answer "where is it" while the node was
// asleep, which is the only time anybody asks.
//
// The transport is not here. A node is claudemon plus `brain --hub <url>
// --token <t>`, an ordinary reconnecting capability PROVIDER, so a woken
// node's sessions arrive as local sessions and every client that already reads
// the fleet reads them unchanged. What this file adds is a registry, a state
// machine, and two bus methods.
//
// THE FLY TOKEN IS NOT IN THIS FILE, not in config.yaml, and not in a flag. It
// is read from nodes.json (0600, config dir) into internal/nodes and never
// leaves it — what a caller receives is nodes.NodeView, an allowlist
// projection built by naming what goes in.

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/flyapi"
	"github.com/djtouchette/workspacer-hub/internal/nodes"
)

// nodeProbeMethod is the liveness probe, and it is deliberately the SAME
// method `workspacer status` uses: brain.info is the one capability only a
// brain ever provides, in every scope. Anything the desktop also registers
// would answer "up" whenever the desktop was running, whatever the node was
// doing.
const nodeProbeMethod = "brain.info"

// nodeStateTopic is the event a client watches instead of polling nodes.list.
const nodeStateTopic = "node.state_changed"

// nodesTrusted is the nodes.wake identity gate.
//
// WAKING A MACHINE SPENDS MONEY, and it keeps spending it until something
// stops the machine — which this hub deliberately cannot do (there is no sleep
// path in v1). That puts it on the far side of the same line jobs.* sits on,
// and above where agents.spawn sits: a spawn costs the caller's own tokens on
// a machine that is already running and already paid for; a wake starts a
// billable machine.
//
// So: host authority only. The host token passes, an OPERATOR-tier pairing
// passes (an operator token authenticates as trusted, which is what makes the
// desktop, /app and /m buttons work), and a view or triage token — the phone
// tiers — is refused here as well as by the tier allowlist that already omits
// this method. A plugin token is refused outright: a plugin's consent dialog
// says "Wake a remote node", and no consent dialog can honestly price a bill.
//
// Always invoked with the capability's OWN literal name; capspec's composition
// bearings verify the nodesTrusted("nodes.<x>", …) call shape in this file.
func nodesTrusted(method string, c bus.CallerIdentity) error {
	if !c.IsTrusted() {
		return fmt.Errorf("%s requires host authority (starting a machine spends money)", method)
	}
	return nil
}

// hubBusProbe is [nodes.BusProbe] over this hub's own bus.
type hubBusProbe struct {
	srv  *bus.Server
	self *busclient.Client
}

// ProbeBrain asks the bus whether a brain is attached, and which node it is.
//
// The three outcomes it must keep apart:
//
//   - answered              → a brain is on the bus.
//   - ErrNoProvider         → nothing is registered. What a stopped node looks
//     like, and the router's own words ("no provider for …") are the
//     definitive signal — a method COUNT cannot tell the hub's own local
//     methods from a live brain.
//   - registered but silent → the zombie: a dead machine's connection still
//     holding the capability slot. Reported as an ordinary error, which the
//     supervisor turns into a strike toward eviction.
//
// A fourth is possible and is neither: our OWN loopback client is not
// connected. That is the hub failing to ask, not the node failing to answer,
// and accusing a provider of silence on that basis would evict a live node.
func (h hubBusProbe) ProbeBrain(ctx context.Context) (nodes.Probe, error) {
	if h.self == nil {
		return nodes.Probe{}, nodes.ErrProbeUnavailable
	}
	raw, err := h.self.Call(ctx, nodeProbeMethod, map[string]any{})
	if err != nil {
		if errors.Is(err, busclient.ErrNotConnected) {
			return nodes.Probe{}, fmt.Errorf("%w: %v", nodes.ErrProbeUnavailable, err)
		}
		if strings.Contains(err.Error(), "no provider") {
			return nodes.Probe{}, nodes.ErrNoProvider
		}
		return nodes.Probe{}, err
	}
	// A brain that was started with WKS_NODE_ID names itself. One that was not
	// returns no node, and the supervisor falls back to "there is only one
	// node registered, so it must be that one". lastExit is the node's own
	// account of how its PREVIOUS run ended — the one thing no cloud API can
	// answer, because a crash-looped machine and a slept one are both
	// `stopped` there.
	var info struct {
		Node     string            `json:"node"`
		LastExit *nodes.ExitRecord `json:"lastExit"`
	}
	_ = json.Unmarshal(raw, &info)
	return nodes.Probe{NodeID: info.Node, LastExit: info.LastExit}, nil
}

func (h hubBusProbe) BrainProviderRegistered() bool {
	_, ok := h.srv.ProviderConnID(nodeProbeMethod)
	return ok
}

func (h hubBusProbe) EvictBrainProvider() bool {
	id, ok := h.srv.ProviderConnID(nodeProbeMethod)
	if !ok {
		return false
	}
	return h.srv.EvictConn(id)
}

// startNodes loads nodes.json, builds the supervisor, registers the two bus
// methods, and starts the reconcile loop. It returns nil (and registers
// nothing) when no registry exists, which is every ordinary desktop install.
func startNodes(ctx context.Context, srv *bus.Server, b *broker.Broker, self *busclient.Client, path, brainScope string) *nodes.Supervisor {
	if path == "" {
		return nil
	}
	entries, err := nodes.LoadFile(path)
	if err != nil {
		// Loud, like peers.json: a typo that silently disables the registry
		// reads to the user as "my remote machine vanished".
		log.Fatalf("nodes: %v", err)
	}
	if len(entries) == 0 {
		return nil
	}
	// Three outcomes, not two. "The hub could not tell" is printed as itself
	// rather than collapsed into either a warning or silence — on Windows the
	// permissions are ACLs and a domain group in the DACL is a question this
	// process cannot answer. See internal/nodes/exposure.go.
	switch exposure, why := nodes.FileExposure(path); exposure {
	case nodes.ExposureLoose:
		log.Printf("nodes: WARNING %s holds a cloud API token that can spend money, and %s", path, why)
	case nodes.ExposureUnknown:
		log.Printf("nodes: NOTE %s holds a cloud API token that can spend money. The hub could NOT confirm only you can read it: %s. This is not a clean bill of health — check it yourself", path, why)
	}
	// The registry's liveness probe is brain.info, and it cannot tell a REMOTE
	// brain from a local one. A hub supervising its own brain would report a
	// stopped node as available, forever.
	if brainScope != "off" {
		log.Printf("nodes: WARNING this hub supervises its own brain (--brain-scope %s) AND has a node registry. "+
			"The liveness probe (brain.info) cannot tell a local brain from a remote one, so node state will be wrong. "+
			"An always-on hub for remote nodes should run --brain-scope off", brainScope)
	}

	clients := map[string]flyapi.Client{}
	for _, n := range entries {
		if !n.Wakeable() {
			log.Printf("nodes: %s has no cloud coordinates — it will be reported but cannot be woken from here", n.ID)
			continue
		}
		tok, err := nodes.ResolveToken(n.Fly)
		if err != nil {
			log.Printf("nodes: %s: %v — it will be reported but cannot be woken from here", n.ID, err)
			continue
		}
		if tok == "" {
			log.Printf("nodes: %s has cloud coordinates but no token (set fly.token, fly.tokenFile, or $%s) — "+
				"it will be reported but cannot be woken from here", n.ID, nodes.FlyTokenEnv)
			continue
		}
		c := flyapi.New(tok)
		c.BaseURL = n.Fly.BaseURL
		clients[n.ID] = c
	}

	sup := nodes.New(nodes.Options{
		Nodes:   entries,
		Bus:     hubBusProbe{srv: srv, self: self},
		Clients: clients,
		Logf:    log.Printf,
		Publish: func(c nodes.Change) {
			b.Publish(event.New(nodeStateTopic, "nodes", map[string]any{
				"node":     c.View,
				"previous": string(c.Previous),
			}))
		},
	})

	// The two bus methods are registered in cmd/hub/main.go, NOT here, and
	// that placement is load-bearing: the brain's headless-completeness guard
	// parses RegisterLocal literals out of main.go, and capspec's composition
	// bearings grep the same file for the nodesTrusted("nodes.<x>", …) call
	// shape. A registration in this file would be invisible to both.

	go sup.Run(ctx)
	log.Printf("nodes: %d node(s) registered from %s (%d wakeable)", len(entries), path, len(clients))
	return sup
}

// nodesList answers `nodes.list`: every registered node with its state. No
// params. See [nodes.NodeView] for what it does — and does not — disclose.
func nodesList(sup *nodes.Supervisor) bus.LocalHandler {
	return func(json.RawMessage) (any, error) { return sup.List(), nil }
}

// nodesWake answers `nodes.wake`: start a stopped node, asynchronously.
//
// It returns the node's view with `state:"waking"` immediately rather than
// blocking until the node is up. That is the whole product decision — a
// client renders a real `waking` state with a disabled composer instead of a
// spinner on a held request — and it is also why there is no dispatch queue
// for input typed during a wake.
func nodesWake(ctx context.Context, sup *nodes.Supervisor) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, p json.RawMessage) (any, error) {
		if err := nodesTrusted("nodes.wake", c); err != nil {
			return nil, err
		}
		var req struct {
			ID string `json:"id"`
		}
		if len(p) > 0 {
			if err := json.Unmarshal(p, &req); err != nil {
				return nil, fmt.Errorf("nodes.wake: %w", err)
			}
		}
		id := strings.TrimSpace(req.ID)
		if id == "" {
			return nil, errors.New("nodes.wake: an `id` naming a registered node is required")
		}
		return sup.Wake(ctx, id)
	}
}
