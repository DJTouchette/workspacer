package bus

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

func TestMigrationBusFixtures(t *testing.T) {
	raw, err := sweepguard.ReadRepoFile("contracts", "hub-bus-cases.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Name  string
			Steps []struct {
				Connect, Send, Expect, Disconnect, Token, Scope, CaptureID string
				Frame                                                      map[string]any
			}
		}
	}
	if err = json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Cases) < 9 {
		t.Fatal("missing migration cases")
	}
	for _, scenario := range fixture.Cases {
		t.Run(scenario.Name, func(t *testing.T) {
			s := NewServer(broker.New())
			s.SetToken("migration-fixture-only")
			s.SetScopedTokenLookup(func(token string) (ScopedIdent, bool) {
				for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator, authtoken.ScopeProvider} {
					if token == "migration-"+string(scope)+"-only" {
						var provides []string
						if scope == authtoken.ScopeProvider {
							provides = []string{"*"}
						}
						return ScopedIdent{Scope: string(scope), Methods: scope.Methods(), Provides: provides}, true
					}
				}
				return ScopedIdent{}, false
			})
			s.RegisterLocal("test.echo", func(p json.RawMessage) (any, error) { return p, nil })
			server := httptest.NewServer(s.Handler())
			defer server.Close()
			clients := map[string]*websocket.Conn{}
			defer func() {
				for _, c := range clients {
					c.CloseNow()
				}
			}()
			ids := map[string]string{}
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			read := func(c *websocket.Conn) map[string]any {
				_, data, err := c.Read(ctx)
				if err != nil {
					t.Fatal(err)
				}
				var v map[string]any
				if err = json.Unmarshal(data, &v); err != nil {
					t.Fatal(err)
				}
				return v
			}
			for index, step := range scenario.Steps {
				if step.Connect != "" {
					token := step.Token
					if token == "" {
						token = "migration-fixture-only"
					}
					c, _, err := websocket.Dial(ctx, "ws"+strings.TrimPrefix(server.URL, "http")+"/bus?token="+token, nil)
					if err != nil {
						t.Fatal(err)
					}
					clients[step.Connect] = c
					hello := read(c)
					scope := step.Scope
					if scope == "" {
						scope = "operator"
					}
					if hello["op"] != "hello" || hello["scope"] != scope {
						t.Fatalf("bad hello %#v", hello)
					}
					continue
				}
				if step.Disconnect != "" {
					clients[step.Disconnect].CloseNow()
					delete(clients, step.Disconnect)
					continue
				}
				if id, ok := step.Frame["id"].(string); ok && strings.HasPrefix(id, "$") {
					if value, ok := ids[id[1:]]; ok {
						step.Frame["id"] = value
					}
				}
				if step.Send != "" {
					data, err := json.Marshal(step.Frame)
					if err != nil {
						t.Fatal(err)
					}
					if err = clients[step.Send].Write(ctx, websocket.MessageText, data); err != nil {
						t.Fatal(err)
					}
					continue
				}
				if step.Expect == "" {
					t.Fatalf("invalid fixture step %d", index)
				}
				actual := read(clients[step.Expect])
				if step.CaptureID != "" {
					id, ok := actual["id"].(string)
					if !ok || id == "" {
						t.Fatal("missing forwarded id")
					}
					ids[step.CaptureID] = id
					step.Frame["id"] = id
				}
				if !reflect.DeepEqual(actual, step.Frame) {
					t.Fatalf("step %d: got %#v, want %#v", index, actual, step.Frame)
				}
			}
		})
	}
}
