package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
	"time"
)

type contextHealthContract struct {
	FormatPctCases []struct {
		Input    float64 `json:"input"`
		Expected string  `json:"expected"`
	} `json:"formatPctCases"`
	UnsupportedProviders []string `json:"unsupportedProviders"`
	CumulativeCodex      struct {
		InputTokens  float64 `json:"inputTokens"`
		OutputTokens float64 `json:"outputTokens"`
		WindowTokens float64 `json:"windowTokens"`
		ThresholdPct float64 `json:"thresholdPct"`
	} `json:"cumulativeCodex"`
}

func loadContextHealthContract(t *testing.T) contextHealthContract {
	t.Helper()
	var c contextHealthContract
	if err := json.Unmarshal(mustReadRepoFile(t, "contracts", "context-health-cases.json"), &c); err != nil {
		t.Fatalf("parse contracts/context-health-cases.json: %v", err)
	}
	if len(c.FormatPctCases) == 0 || len(c.UnsupportedProviders) == 0 {
		t.Fatal("context-health contract decoded empty; a key was renamed or the corpus was gutted")
	}
	return c
}

func TestContextHealthFormattingMatchesDesktopContract(t *testing.T) {
	for _, c := range loadContextHealthContract(t).FormatPctCases {
		if got := formatContextPct(c.Input); got != c.Expected {
			t.Errorf("formatContextPct(%v) = %q, want %q", c.Input, got, c.Expected)
		}
	}
}

func TestContextWatchRejectsUnsupportedProvidersWithoutUsingSlots(t *testing.T) {
	c := loadContextHealthContract(t)
	for _, provider := range c.UnsupportedProviders {
		t.Run(provider, func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			worker := fmt.Sprintf(`{"session_id":"worker","cwd":"/w/p","mode":"responding","provider":%q}`, provider)
			reg := fleetReg(t, srv.URL,
				map[string]string{"mgr": row("mgr", "/w", "input"), "worker": worker},
				map[string]spawnMeta{"worker": {ParentSessionID: "mgr"}})
			_, err := reg.handle(context.Background(), "agents.notifyWhen", json.RawMessage(`{"sessionId":"worker","contextUsedPct":80}`))
			if err == nil || !strings.Contains(err.Error(), "contextUsedPct is unavailable for provider "+provider) {
				t.Fatalf("unsupported provider refusal = %v", err)
			}
			if len(reg.watches) != 0 {
				t.Fatalf("refused %s watch consumed %d slot(s)", provider, len(reg.watches))
			}
		})
	}
}

func TestCumulativeCodexContractCannotFireContextWatch(t *testing.T) {
	c := loadContextHealthContract(t).CumulativeCodex
	if c.InputTokens/c.WindowTokens*100 <= c.ThresholdPct {
		t.Fatal("fixture no longer mutation-proves the cumulative fallback")
	}
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	worker := fmt.Sprintf(`{"session_id":"worker","cwd":"/w/p","mode":"responding","provider":"codex","status_line":{"total_input_tokens":%v,"total_output_tokens":%v,"context_window_size":%v}}`, c.InputTokens, c.OutputTokens, c.WindowTokens)
	reg := fleetReg(t, srv.URL,
		map[string]string{"mgr": row("mgr", "/w", "input"), "worker": worker},
		map[string]spawnMeta{"worker": {ParentSessionID: "mgr"}})
	ctx := context.Background()
	if _, err := reg.handle(ctx, "agents.notifyWhen", json.RawMessage(fmt.Sprintf(`{"sessionId":"worker","contextUsedPct":%v}`, c.ThresholdPct))); err != nil {
		t.Fatal(err)
	}
	reg.sweepThresholds(ctx, time.Now())
	if got := len(rec.calls("/sessions/mgr/message")); got != 0 {
		t.Fatalf("cumulative-only Codex usage fired %d context wake(s)", got)
	}
	if len(reg.watches) != 1 {
		t.Fatalf("missing telemetry should leave one watch waiting, got %d", len(reg.watches))
	}
}
