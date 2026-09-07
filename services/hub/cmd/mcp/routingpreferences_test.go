package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"github.com/djtouchette/workspacer-hub/internal/routing"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type routingAuthTransport struct{ token string }

func (a routingAuthTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	r = r.Clone(r.Context())
	if a.token != "" {
		r.Header.Set("Authorization", "Bearer "+a.token)
	}
	// Proves a caller cannot manufacture the facade's internal host marker.
	r.Header.Set(routingHostHeader, "authenticated")
	return http.DefaultTransport.RoundTrip(r)
}
func TestRoutingMCPHostGateAndSelectModelConsumer(t *testing.T) {
	for _, transport := range []string{"http", "sse"} {
		t.Run(transport, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
			defer cancel()
			svc := routing.New(filepath.Join(t.TempDir(), "routing.yaml"), nil)
			hub := bus.NewServer(broker.New())
			hub.SetToken("hub-host")
			hub.RegisterLocalIdent("routing.preferences.get", func(c bus.CallerIdentity, _ json.RawMessage) (any, error) { return svc.Preferences(), nil })
			writes := 0
			hub.RegisterLocalIdent("routing.preferences.save", func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
				if !c.AuthenticatedHost || c.Scope != "operator" {
					return nil, fmt.Errorf("host only")
				}
				var req routing.PreferencesRequest
				if e := routing.DecodePreferences(raw, &req); e != nil {
					return nil, e
				}
				writes++
				return svc.UpdatePreferences(req, "save")
			})
			hub.RegisterLocalIdent("routing.select", func(_ bus.CallerIdentity, raw json.RawMessage) (any, error) {
				var req routing.Request
				if e := json.Unmarshal(raw, &req); e != nil {
					return nil, e
				}
				return routing.Select(svc.Matrix(), limits.Snapshot{}, nil, nil, time.Now(), req), nil
			})
			hs := httptest.NewServer(hub.Handler())
			defer hs.Close()
			client := busclient.New(strings.Replace(hs.URL, "http", "ws", 1)+"/bus", "hub-host")
			go client.Run(ctx)
			store, operator := mintTestToken(t, authtoken.ScopeOperator)
			gate := &authGate{static: "facade-host", store: store, untokened: untokenedDeny}
			cache := newServerCache(client, newPluginCatalog(client), tierServers(client))
			httpServer := httptest.NewServer(newMux(cache, client, gate))
			defer httpServer.Close()
			for _, tc := range []struct {
				token   string
				allowed bool
			}{{operator, false}, {"facade-host", true}} {
				hc := &http.Client{Transport: routingAuthTransport{tc.token}}
				var tr mcp.Transport
				if transport == "http" {
					tr = &mcp.StreamableClientTransport{Endpoint: httpServer.URL + "/mcp", HTTPClient: hc}
				} else {
					tr = &mcp.SSEClientTransport{Endpoint: httpServer.URL + "/sse", HTTPClient: hc}
				}
				mc := mcp.NewClient(&mcp.Implementation{Name: "routing-test", Version: "1"}, nil)
				session, e := mc.Connect(ctx, tr, nil)
				if e != nil {
					t.Fatal(e)
				}
				get, e := session.CallTool(ctx, &mcp.CallToolParams{Name: "routing_preferences_get", Arguments: map[string]any{}})
				if e != nil || get.IsError {
					t.Fatalf("get %v %v", get, e)
				}
				var view routing.PreferencesView
				if e = json.Unmarshal([]byte(textOf(get)), &view); e != nil {
					t.Fatal(e)
				}
				if view.Configurable != tc.allowed {
					t.Fatalf("configurable ignored facade authority: %v", view.Configurable)
				}
				save, e := session.CallTool(ctx, &mcp.CallToolParams{Name: "routing_preferences_save", Arguments: map[string]any{"baseRevision": view.Revision, "patch": map[string]any{"activeProfile": "codex_only", "roles": map[string]string{"scout": "cheap"}}}})
				if e != nil {
					t.Fatal(e)
				}
				if save.IsError == tc.allowed {
					t.Fatalf("host gate allowed=%v: %s", tc.allowed, textOf(save))
				}
				if tc.allowed {
					choice, e := session.CallTool(ctx, &mcp.CallToolParams{Name: "select_model", Arguments: map[string]any{"role": "scout"}})
					if e != nil || choice.IsError {
						t.Fatalf("select %v %v", choice, e)
					}
					var d routing.Decision
					_ = json.Unmarshal([]byte(textOf(choice)), &d)
					if d.Model != "gpt-5.6-luna" || d.Capability != "cheap" {
						t.Fatalf("actual select_model ignored save %+v", d)
					}
				}
				session.Close()
			}
			if writes != 1 {
				t.Fatalf("operator crossed facade host gate: %d writes", writes)
			}
		})
	}
}
