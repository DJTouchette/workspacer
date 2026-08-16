// Plugin-contributed tools.
//
// Installed workspacer plugins may declare `tools` in their manifest — MCP
// tool definitions bound to bus methods the plugin itself answers (its
// `provides`). The hub serves the CONSENTED surface via the hub-local
// `plugins.tools` method (pin-narrowed, so nothing is listed the bus would
// refuse to let the plugin register); this file polls that catalog and grafts
// the tools onto per-token servers.
//
// Opt-in per session, never ambient: a plugin tool is exposed only to a
// request whose scoped token lists the plugin (authtoken.Record.Plugins) —
// installed-plugin tools do not tax every connected agent's context, and the
// untokened loopback default gets none.
package main

import (
	"context"
	"encoding/json"
	"log"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// pluginToolDef mirrors plugin.ToolDef's wire shape. A deliberately minimal
// copy (like busclient.frame) so the facade doesn't import the plugin package.
type pluginToolDef struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema,omitempty"`
	Method      string          `json:"method"`
}

// pluginTools is one plugin's consented tool surface (plugin.PluginTools).
type pluginTools struct {
	PluginID string          `json:"pluginId"`
	Tools    []pluginToolDef `json:"tools"`
}

// catalogPollInterval is how often the facade re-asks the hub for the
// consented tool surface. Install/enable/reload of a plugin shows up within
// one interval; a token's plugin GRANTS apply instantly (they live in
// tokens.json, resolved per request).
const catalogPollInterval = 15 * time.Second

// pluginCatalog is the facade's view of the hub's consented plugin-tool
// surface, refreshed by polling `plugins.tools` over the trusted bus
// connection. gen increments only when the surface actually changes, so the
// server cache can key on it.
type pluginCatalog struct {
	c *busclient.Client

	mu   sync.Mutex
	gen  int
	raw  string // last marshaled surface, for change detection
	byID map[string][]pluginToolDef
}

func newPluginCatalog(c *busclient.Client) *pluginCatalog {
	return &pluginCatalog{c: c, byID: map[string][]pluginToolDef{}}
}

// run polls until ctx ends. Errors are quiet retries — the hub may not be up
// yet, or may be a version without plugins.tools, in which case the catalog
// simply stays empty and no plugin tools are ever advertised.
func (pc *pluginCatalog) run(ctx context.Context) {
	t := time.NewTicker(catalogPollInterval)
	defer t.Stop()
	for {
		pc.refresh(ctx)
		select {
		case <-ctx.Done():
			return
		case <-t.C:
		}
	}
}

func (pc *pluginCatalog) refresh(ctx context.Context) {
	callCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	res, err := pc.c.Call(callCtx, "plugins.tools", nil)
	if err != nil {
		return
	}
	var list []pluginTools
	if json.Unmarshal(res, &list) != nil {
		return
	}
	byID := make(map[string][]pluginToolDef, len(list))
	for _, p := range list {
		if p.PluginID != "" && len(p.Tools) > 0 {
			byID[p.PluginID] = p.Tools
		}
	}
	raw := string(res)
	pc.mu.Lock()
	if raw != pc.raw {
		pc.raw = raw
		pc.byID = byID
		pc.gen++
		log.Printf("plugin tool catalog updated: %d plugin(s)", len(byID))
	}
	pc.mu.Unlock()
}

// snapshot returns the current surface and its generation.
func (pc *pluginCatalog) snapshot() (map[string][]pluginToolDef, int) {
	pc.mu.Lock()
	defer pc.mu.Unlock()
	return pc.byID, pc.gen
}

// serverCache hands out the MCP server for a resolved token: the plain tier
// server when the token grants no plugins, or a tier+plugin-tools server built
// on demand and cached by (scope, granted plugins, catalog generation). The
// cache is flushed whenever the catalog generation moves, so a plugin
// reload/uninstall retires its tools within one poll interval.
type serverCache struct {
	c       *busclient.Client
	catalog *pluginCatalog
	base    map[authtoken.Scope]*mcp.Server

	mu    sync.Mutex
	gen   int
	built map[string]*mcp.Server
}

func newServerCache(c *busclient.Client, catalog *pluginCatalog, base map[authtoken.Scope]*mcp.Server) *serverCache {
	return &serverCache{c: c, catalog: catalog, base: base, built: map[string]*mcp.Server{}}
}

// serverFor returns the server a resolved token record should be served.
func (sc *serverCache) serverFor(rec authtoken.Record) *mcp.Server {
	if len(rec.Plugins) == 0 {
		if s := sc.base[rec.Scope]; s != nil {
			return s
		}
		return newDeniedServer() // unknown scope record: fail closed, no tools
	}
	if sc.base[rec.Scope] == nil {
		return newDeniedServer()
	}
	byID, gen := sc.catalog.snapshot()
	granted := grantedPlugins(rec.Plugins, byID)
	key := string(rec.Scope) + "|" + strings.Join(granted, ",") + "|" + strconv.Itoa(gen)

	sc.mu.Lock()
	defer sc.mu.Unlock()
	if sc.gen != gen {
		sc.built = map[string]*mcp.Server{}
		sc.gen = gen
	}
	if s := sc.built[key]; s != nil {
		return s
	}
	var defs []grantedPluginTools
	for _, id := range granted {
		defs = append(defs, grantedPluginTools{PluginID: id, Tools: byID[id]})
	}
	s := newServerWithPlugins(sc.c, rec.Scope, defs)
	sc.built[key] = s
	return s
}

// grantedPlugins resolves a token's plugin grant list against the catalog:
// exact ids that exist, or every catalog plugin when the grant is "*"
// (supervisor convenience). Sorted for a stable cache key.
func grantedPlugins(grants []string, byID map[string][]pluginToolDef) []string {
	set := map[string]bool{}
	for _, g := range grants {
		if g == "*" {
			for id := range byID {
				set[id] = true
			}
			continue
		}
		if _, ok := byID[g]; ok {
			set[g] = true
		}
	}
	out := make([]string, 0, len(set))
	for id := range set {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

// grantedPluginTools is one plugin's tools as granted to a specific token.
type grantedPluginTools struct {
	PluginID string
	Tools    []pluginToolDef
}

// mcpNameSanitizer collapses everything outside [a-z0-9] to '_' so a plugin id
// ("djtouchette.jira") becomes a legal MCP tool-name prefix.
var mcpNameSanitizer = regexp.MustCompile(`[^a-z0-9]+`)

func sanitizeMCPName(id string) string {
	return strings.Trim(mcpNameSanitizer.ReplaceAllString(strings.ToLower(id), "_"), "_")
}

// addPluginTool registers one plugin-contributed tool on a build's server. The
// tool name is the FULL sanitized plugin id + tool name — deterministic and
// collision-free across plugins, at the cost of length. The handler forwards
// the raw arguments object as the bus call's params; the hub routes it to the
// plugin's registered provider connection.
func addPluginTool(b *build, pluginID string, t pluginToolDef) {
	name := sanitizeMCPName(pluginID) + "_" + t.Name
	schema := t.InputSchema
	if len(schema) == 0 {
		schema = json.RawMessage(`{"type":"object"}`)
	}
	// The hub validates this at manifest load, but the SDK PANICS on a non-
	// object schema, so re-guard here rather than letting a hub/facade version
	// skew take the whole facade down.
	var m map[string]any
	if json.Unmarshal(schema, &m) != nil || m["type"] != "object" {
		log.Printf("plugin %s: skipping tool %q — inputSchema is not a JSON Schema object", pluginID, t.Name)
		return
	}
	method := t.Method
	b.tools = append(b.tools, toolInfo{Name: name, Desc: t.Description, Method: method, Group: "plugins"})
	b.s.AddTool(&mcp.Tool{Name: name, Description: t.Description, InputSchema: schema},
		func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			var params any
			if len(req.Params.Arguments) > 0 {
				if err := json.Unmarshal(req.Params.Arguments, &params); err != nil {
					return &mcp.CallToolResult{
						IsError: true,
						Content: []mcp.Content{&mcp.TextContent{Text: "invalid arguments: " + err.Error()}},
					}, nil
				}
			}
			res, _, _ := forward(ctx, b.c, method, params)
			return res, nil
		})
}
