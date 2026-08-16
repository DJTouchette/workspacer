// Cross-hub (federated) fleet support.
//
// The local hub can link outbound to peer hubs (see internal/federation and
// docs/hub-federation.md): peer capabilities become callable as
// `hub:<peer>/<method>` and the hub-local `federation.peers` method lists the
// links. This file makes the facade fleet-wide over that substrate:
//
//   - the fleet listing tools (list_agents, list_snapshots) merge every
//     connected peer's rows into the local result, tagging each remote row
//     with its hub name (local rows stay untagged);
//   - the per-session tools accept an optional `hub` input that routes the
//     call as `hub:<hub>/<method>`, so a supervisor can drive a remote
//     session exactly like a local one.
//
// Tier correctness: which tools exist per tier is unchanged — every registrar
// here still gates on the BARE method against the tier's allowlist
// (b.allowed), the same first line every tool has, and the hub router applies
// the identical bare-method tier check server-side for scoped bus tokens. The
// facade's own bus connection is TRUSTED, so its qualified calls pass the
// local router unconditionally; the peer-side federation link token is the
// real ceiling on everything forwarded (rpc.go federatedCall documents the
// same split).
package main

import (
	"context"
	"encoding/json"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// peerListBudget bounds each peer's contribution to a fleet-wide call. The
// federation forward budget is 25s; a listing should not hang the whole tool
// on one wedged peer for that long — a peer that can't answer in this window
// simply costs its own rows.
const peerListBudget = 10 * time.Second

// peerInfo mirrors federation.PeerInfo's wire shape — a deliberately minimal
// copy (like busclient.frame) so the facade doesn't import the federation
// package.
type peerInfo struct {
	Name      string `json:"name"`
	Connected bool   `json:"connected"`
}

// connectedPeers asks the local hub which federation links are currently up.
// Every failure collapses to nil — an older hub or federation-off setup
// answers "no provider"/"federation not configured", and a blip is just a
// blip — so callers merge nothing and the fleet tools behave exactly as the
// per-hub v1 did.
func connectedPeers(ctx context.Context, c *busclient.Client) []string {
	pctx, cancel := context.WithTimeout(ctx, peerListBudget)
	defer cancel()
	res, err := c.Call(pctx, "federation.peers", nil)
	if err != nil {
		return nil
	}
	var peers []peerInfo
	if json.Unmarshal(res, &peers) != nil {
		return nil
	}
	var out []string
	for _, p := range peers {
		if p.Connected && p.Name != "" {
			out = append(out, p.Name)
		}
	}
	return out
}

// mergeFleetRows widens a local listing result with every connected peer's
// rows. The result stays passthrough JSON text: local rows are appended
// byte-for-byte untouched (and untagged), each remote row gains a
// `"hub":"<peer>"` field naming the machine it lives on. A per-peer error —
// link down mid-call, no provider on the peer, an unparseable answer — costs
// that peer's rows, never the call. A local result that isn't a JSON array is
// returned verbatim (nothing sane to merge into).
func mergeFleetRows(ctx context.Context, c *busclient.Client, method string, params any, local json.RawMessage) json.RawMessage {
	var localRows []json.RawMessage
	if json.Unmarshal(local, &localRows) != nil {
		return local
	}
	peers := connectedPeers(ctx, c)
	if len(peers) == 0 {
		return local
	}

	// Peers answer in parallel so N peers cost one peerListBudget, not N.
	tagged := make([][]json.RawMessage, len(peers))
	var wg sync.WaitGroup
	for i, name := range peers {
		wg.Add(1)
		go func(i int, name string) {
			defer wg.Done()
			pctx, cancel := context.WithTimeout(ctx, peerListBudget)
			defer cancel()
			res, err := c.Call(pctx, "hub:"+name+"/"+method, params)
			if err != nil {
				return
			}
			var rows []map[string]any
			if json.Unmarshal(res, &rows) != nil {
				return
			}
			out := make([]json.RawMessage, 0, len(rows))
			for _, row := range rows {
				row["hub"] = name
				b, err := json.Marshal(row)
				if err != nil {
					continue
				}
				out = append(out, b)
			}
			tagged[i] = out
		}(i, name)
	}
	wg.Wait()

	merged := localRows
	for _, rows := range tagged {
		merged = append(merged, rows...)
	}
	out, err := json.Marshal(merged)
	if err != nil {
		return local
	}
	if merged == nil {
		// json.Marshal(nil slice) is "null"; an empty fleet is an empty array.
		return json.RawMessage("[]")
	}
	return out
}

// forwardFleet is forward() for the fleet-wide listing tools: the local call's
// error handling is identical (a dead hub is a tool error, never a hang), and
// a successful local result is widened with the connected peers' rows.
func forwardFleet(ctx context.Context, c *busclient.Client, method string, params any) (*mcp.CallToolResult, any, error) {
	res, err := c.Call(ctx, method, params)
	if err != nil {
		return &mcp.CallToolResult{
			IsError: true,
			Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}},
		}, nil, nil
	}
	text := string(mergeFleetRows(ctx, c, method, params, res))
	if text == "" || text == "null" {
		text = "ok"
	}
	return &mcp.CallToolResult{
		Content: []mcp.Content{&mcp.TextContent{Text: text}},
	}, nil, nil
}

// addFleetTool registers a fleet-wide listing tool: same registration contract
// as addTool (bare-method tier gate, help registry), fleet-merged forwarding.
func addFleetTool[In any](b *build, name, desc, method string) {
	if !b.allowed(method) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: method, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in In) (*mcp.CallToolResult, any, error) {
			return forwardFleet(ctx, b.c, method, in)
		})
}

// hubArg is embedded in every per-session tool input to add the optional
// federation-routing field. encoding/json (and the SDK's schema inference)
// promote the embedded field, so it appears as a plain optional `hub` property
// on the tool's input schema.
type hubArg struct {
	Hub string `json:"hub,omitempty" jsonschema:"the peer hub this session lives on — the hub field on its list_agents row; omit for a local session"`
}

// takeHub returns the routing target and strips it from the params, so the
// forwarded capability sees exactly the fields it expects — the peer's
// provider has no `hub` param and must not receive a stray one.
func (h *hubArg) takeHub() string {
	peer := h.Hub
	h.Hub = ""
	return peer
}

// hubRouted is what addHubTool needs from an input struct: hand over (and
// clear) the optional hub field. hubArg implements it; spawnAgentIn carries
// its own hub field (different description) and implements it directly.
type hubRouted interface{ takeHub() string }

// addHubTool registers a per-session tool whose input may name the peer hub
// the session lives on. Bare inputs behave exactly as before (local call);
// with hub set, the method forwards as `hub:<hub>/<method>` through the local
// hub's federation router. See the package comment for why the tier story is
// unchanged: the tool only exists in tiers whose scope admits the bare method,
// and the peer-side link token clamps what the forwarded call may do.
func addHubTool[In any, P interface {
	*In
	hubRouted
}](b *build, name, desc, method string) {
	if !b.allowed(method) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: method, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in In) (*mcp.CallToolResult, any, error) {
			m := method
			if peer := P(&in).takeHub(); peer != "" {
				m = "hub:" + peer + "/" + method
			}
			return forward(ctx, b.c, m, in)
		})
}
