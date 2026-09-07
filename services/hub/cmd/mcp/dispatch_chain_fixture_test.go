package main

import (
	"context"
	"encoding/json"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"github.com/djtouchette/workspacer-hub/internal/routing"
	"io"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// TestDesktopDispatchChainFixture is a subprocess server for the desktop's
// test:dispatch-chain command (also run in CI). Like spawnGrantSession it uses
// the real bus, but its provider is the production TypeScript desktop handler,
// and requests enter through the real HTTP credential gate, not connectTo or
// a build.caller stub. No hub main/brain/daemon supervisor is started.
func TestDesktopDispatchChainFixture(t *testing.T) {
	if os.Getenv("WKS_DISPATCH_CHAIN_FIXTURE") != "1" {
		return // The cross-language assertions live in dispatchChain.integration.ts.
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	configDir := authtoken.ConfigDir()
	tokensPath := filepath.Join(configDir, "tokens.json")
	for _, label := range []string{"session:manager-current", "session:manager-other", "pairing"} {
		if _, err := authtoken.Mint(tokensPath, authtoken.ScopeOperator, label); err != nil {
			t.Fatal("mint synthetic fixture credential failed")
		}
	}
	store := authtoken.NewStore(tokensPath)
	srv := bus.NewServer(broker.New())
	// Public fixture constants, never read from the user's environment or files.
	srv.SetToken("dispatch-chain-synthetic-host")
	matrix, err := routing.Defaults()
	if err != nil {
		t.Fatal(err)
	}
	matrix.ActiveProfile = "codex_only"
	matrix.Ceilings = map[string]routing.Ceiling{"default": {MaxCapability: "frontier", MaxToolScope: "view"}}
	srv.RegisterLocal("routing.select", func(raw json.RawMessage) (any, error) {
		var req routing.Request
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		decision := routing.Select(matrix, limits.Snapshot{}, nil, nil, time.Now(), req)
		decision.DecisionID = routing.NewDecisionID()
		return decision, nil
	})
	srv.SetSpawnCeiling(func(req bus.SpawnCeilingRequest) bus.SpawnCeilingVerdict {
		// Legacy identity-only assertions below declare no role; workflow tests use real routing.
		if req.Role == "" {
			return bus.SpawnCeilingVerdict{}
		}
		v := matrix.CheckSpawn(routing.SpawnRequest{CanonicalCwd: req.CanonicalCwd, Capability: req.Capability, Role: req.Role, Resuming: req.Resuming, ResumeSessionID: req.ResumeSessionID, ToolScope: req.ToolScope, Provider: req.Provider, Model: req.Model, Effort: req.Effort})
		return bus.SpawnCeilingVerdict{Key: v.Key, MaxCapability: v.MaxCapability, MaxToolScope: v.MaxToolScope, CapabilityRefused: v.CapabilityRefused, Capability: v.Capability, ToolScopeRefused: v.ToolScopeRefused, ToolScope: v.ToolScope, Provider: v.Provider, Model: v.Model, Effort: v.Effort, ResumeRefused: v.ResumeRefused, FreshCapability: v.FreshCapability, Denied: v.Denied, Because: v.Because}
	}, nil)

	srv.SetScopedTokenLookup(func(token string) (bus.ScopedIdent, bool) {
		rec, ok := store.Lookup(token)
		return bus.ScopedIdent{Scope: string(rec.Scope), Methods: rec.Scope.Methods(), Label: rec.Label}, ok
	})
	srv.RegisterPluginToken("dispatch-chain-synthetic-plugin", "fixture.plugin",
		[]capspec.Grant{{Method: "agents.spawn"}}, capspec.EventGrants{})
	hub := httptest.NewServer(srv.Handler())
	defer hub.Close()
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	client := busclient.New(busURL, "dispatch-chain-synthetic-host")
	go client.Run(ctx)
	gate := &authGate{store: store, untokened: untokenedDeny}
	catalog := newPluginCatalog(client)
	// No plugin polling is needed: this test calls built-in spawn_agent only.
	mux := newMux(newServerCache(client, catalog, tierServers(client)), client, gate)
	facade := httptest.NewServer(servedHandler("127.0.0.1", mux))
	defer facade.Close()
	if err := json.NewEncoder(os.Stdout).Encode(map[string]string{
		"busURL": busURL, "facadeURL": facade.URL,
	}); err != nil {
		t.Fatal("write fixture readiness failed")
	}
	// EOF shuts down ONLY these scratch servers; even an abruptly killed parent
	// cannot orphan the fixture. The Go test timeout is an independent backstop.
	_, _ = io.Copy(io.Discard, os.Stdin)
}
