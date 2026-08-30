package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"github.com/djtouchette/workspacer-hub/internal/routing"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// TestSelectModelCarriesExpectedWork is the "prove the value arrives" check for
// §15's phase counts, and it exists because the review found the weighted
// forecast path unreachable: `forecast_weights` shipped in the matrix,
// limits.DemandFromWork implemented it, routing.Request accepted it — and the
// one surface supervised agents actually hold, the select_model MCP tool, had
// no field to put it in. Live configuration no caller can reach is dead
// configuration.
//
// The chain proved here is the whole chain the facade is responsible for: the
// tool ADVERTISES the field, addTool forwards the input struct verbatim as the
// call's params (see forward), and routing.Request decodes those bytes on the
// hub side into the []limits.Work that limits.Forecast weights. A json tag that
// disagreed at any hop would leave the field advertised and silently dropped,
// which is exactly the shape of bug this test is for.
func TestSelectModelCarriesExpectedWork(t *testing.T) {
	t.Run("the tool advertises the field", func(t *testing.T) {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()

		client := busclient.New("ws://127.0.0.1:0/bus", "")
		// Operator: routing.select is deliberately in no scoped tier, so this
		// is the only tier that carries the tool at all.
		server := newServer(client, authtoken.ScopeOperator)
		cT, sT := mcp.NewInMemoryTransports()
		if _, err := server.Connect(ctx, sT, nil); err != nil {
			t.Fatalf("server connect: %v", err)
		}
		mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
		cs, err := mc.Connect(ctx, cT, nil)
		if err != nil {
			t.Fatalf("client connect: %v", err)
		}
		defer cs.Close()

		tools, err := cs.ListTools(ctx, nil)
		if err != nil {
			t.Fatalf("ListTools: %v", err)
		}
		var schema []byte
		for _, tl := range tools.Tools {
			if tl.Name != "select_model" {
				continue
			}
			if schema, err = json.Marshal(tl.InputSchema); err != nil {
				t.Fatalf("marshal input schema: %v", err)
			}
		}
		if len(schema) == 0 {
			t.Fatal("the operator tier has no select_model tool at all")
		}
		for _, want := range []string{"expectedWork", "phase", "count"} {
			if !strings.Contains(string(schema), want) {
				t.Errorf("select_model's input schema does not offer %q — an agent cannot pass what the schema does not name:\n%s", want, schema)
			}
		}
	})

	t.Run("the field survives the wire into the weighted forecast", func(t *testing.T) {
		// addTool forwards the input STRUCT as the call params, so the bytes
		// the hub decodes are exactly this marshal.
		in := routingSelectIn{
			Role: "implementer",
			ExpectedWork: []routingWorkIn{
				{Phase: "implementation", Count: 2},
				{Phase: "review", Count: 4},
				{Phase: "haruspicy", Count: 3},
			},
		}
		raw, err := json.Marshal(in)
		if err != nil {
			t.Fatalf("marshal: %v", err)
		}
		var req routing.Request
		if err := json.Unmarshal(raw, &req); err != nil {
			t.Fatalf("routing.Request does not decode the facade's own params: %v\n%s", err, raw)
		}
		if len(req.ExpectedWork) != 3 {
			t.Fatalf("expectedWork did not survive the wire: %+v (params were %s)", req.ExpectedWork, raw)
		}
		if req.ExpectedWork[0].Phase != "implementation" || req.ExpectedWork[0].Count != 2 {
			t.Fatalf("the phase/count field names disagree across the hop: %+v", req.ExpectedWork[0])
		}

		m, err := routing.Load("", nil)
		if err != nil {
			t.Fatalf("load the shipped matrix: %v", err)
		}
		d := limits.Forecast(req.ForecastDemandBeforeResetPct, req.ExpectedWork, m.ForecastWeights)
		// implementation 2 x 4 + review 4 x 2 = 16, on the shipped weights.
		if d.Units != 16 {
			t.Errorf("weighted units = %v, want 16 — the matrix's forecast_weights are not reaching the facade's work counts", d.Units)
		}
		if d.Known {
			t.Error("work units became a KNOWN percentage: limits/forecast.go is explicit that there is no cost model to convert them, and inventing one is the number-that-looks-like-evidence this layer exists to avoid")
		}
		if len(d.UnweightedPhases) != 1 || d.UnweightedPhases[0] != "haruspicy" {
			t.Errorf("a phase the matrix has no weight for was not reported back: %+v", d.UnweightedPhases)
		}
	})
}
