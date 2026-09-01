package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestNotifyWhenSchemaExposesConfirmedContextHealth(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cs := connectToolClient(t, ctx, func(b *build) {
		addTool[notifyWhenIn](b, "notify_when", "watch", "agents.notifyWhen")
	})
	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, tool := range tools.Tools {
		if tool.Name != "notify_when" {
			continue
		}
		raw, _ := json.Marshal(tool.InputSchema)
		text := string(raw)
		for _, want := range []string{"contextUsedPct", "runtime-confirmed", "(0,100]", "cache-inclusive", "not active-context health"} {
			if !strings.Contains(text, want) {
				t.Errorf("notify_when schema missing %q: %s", want, text)
			}
		}
		return
	}
	t.Fatal("notify_when was not registered")
}
