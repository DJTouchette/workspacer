// Command hub-reference exposes the existing bus in an isolated compatibility
// fixture. It starts no agents, plugins, brain, or persistent services. Closing
// stdin ends the fixture. Delete this command with Go after migration parity.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"sort"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

func main() {
	snapshot := flag.Bool("snapshot", false, "export existing capability/topic/route vocabulary")
	flag.Parse()
	if *snapshot {
		methods := map[string]bool{}
		for m := range capspec.PathParam {
			methods[m] = true
		}
		for _, m := range capspec.InertMethods() {
			methods[m] = true
		}
		for _, m := range capspec.UnscopedMethods() {
			methods[m] = true
		}
		names := make([]string, 0, len(methods))
		for m := range methods {
			names = append(names, m)
		}
		sort.Strings(names)
		encoder := json.NewEncoder(os.Stdout)
		encoder.SetIndent("", "  ")
		scopes := map[string][]string{}
		for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator, authtoken.ScopeProvider} {
			scopes[string(scope)] = scope.Methods()
		}
		if err := encoder.Encode(map[string]any{"version": 1, "methods": names, "topics": capspec.EventTopics(), "routes": capspec.HTTPRoutes(), "scopes": scopes, "spawnKeys": bus.SpawnParamKeys()}); err != nil {
			panic(err)
		}
		return
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		panic(err)
	}
	s := bus.NewServer(broker.New())
	s.SetToken("migration-fixture-only")
	s.SetScopedTokenLookup(func(token string) (bus.ScopedIdent, bool) {
		for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator, authtoken.ScopeProvider} {
			if token == "migration-"+string(scope)+"-only" {
				var provides []string
				if scope == authtoken.ScopeProvider {
					provides = []string{"*"}
				}
				return bus.ScopedIdent{Scope: string(scope), Methods: scope.Methods(), Provides: provides}, true
			}
		}
		return bus.ScopedIdent{}, false
	})
	s.RegisterLocal("test.echo", func(p json.RawMessage) (any, error) { return p, nil })
	server := &http.Server{Handler: s.Handler(), ReadHeaderTimeout: 5 * time.Second}
	go func() { _, _ = io.Copy(io.Discard, os.Stdin); _ = server.Shutdown(context.Background()) }()
	fmt.Printf("{\"address\":%q}\n", listener.Addr().String())
	if err := server.Serve(listener); err != nil && err != http.ErrServerClosed {
		panic(err)
	}
}
