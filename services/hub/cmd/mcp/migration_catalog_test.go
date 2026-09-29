package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// The exported data is presentation/schema, not a claim of tool behavior
// parity. Updating is explicit; ordinary tests only compare the reference.
func TestRustMigrationToolCatalog(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	catalog := map[string][]*mcp.Tool{}
	type group struct {
		Name  string   `json:"name"`
		Tools []string `json:"tools"`
	}
	groups := map[string][]group{}
	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator} {
		server := newServer(busclient.New("ws://127.0.0.1:0/bus", ""), scope)
		clientTransport, serverTransport := mcp.NewInMemoryTransports()
		ss, err := server.Connect(ctx, serverTransport, nil)
		if err != nil {
			t.Fatal(err)
		}
		client := mcp.NewClient(&mcp.Implementation{Name: "migration", Version: "1"}, nil)
		cs, err := client.Connect(ctx, clientTransport, nil)
		if err != nil {
			t.Fatal(err)
		}
		result, err := cs.ListTools(ctx, nil)
		if err != nil {
			t.Fatal(err)
		}
		catalog[string(scope)] = result.Tools
		overview, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "help", Arguments: map[string]any{}})
		if err != nil {
			t.Fatal(err)
		}
		for _, content := range overview.Content {
			if text, ok := content.(*mcp.TextContent); ok {
				for _, line := range strings.Split(text.Text, "\n") {
					if !strings.HasPrefix(line, "- ") {
						continue
					}
					name, names, ok := strings.Cut(strings.TrimPrefix(line, "- "), ": ")
					if ok {
						groups[string(scope)] = append(groups[string(scope)], group{Name: name, Tools: strings.Split(names, ", ")})
					}
				}
			}
		}
		cs.Close()
		ss.Close()
	}
	for name, value := range map[string]any{
		"mcp-tools.json": catalog,
		"mcp-help.json":  map[string]any{"guidance": groupGuidance, "groups": groups},
	} {
		encoded, err := json.MarshalIndent(value, "", "  ")
		if err != nil {
			t.Fatal(err)
		}
		if os.Getenv("WKS_UPDATE_RUST_MIGRATION_ASSETS") == "1" {
			path := filepath.Join("..", "..", "..", "hub-rs", "assets", name)
			if err = os.WriteFile(path, append(encoded, '\n'), 0o644); err != nil {
				t.Fatal(err)
			}
		}
		raw, err := sweepguard.ReadRepoFile("services", "hub-rs", "assets", name)
		if err != nil {
			t.Fatal(err)
		}
		var actual, expected any
		if err = json.Unmarshal(raw, &actual); err != nil {
			t.Fatal(err)
		}
		if err = json.Unmarshal(encoded, &expected); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(actual, expected) {
			t.Fatalf("Rust MCP asset %s drifted from Go; run make hub-mcp-catalog after reviewing the change", name)
		}
	}
}
