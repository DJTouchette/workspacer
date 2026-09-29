package routing

import (
	"encoding/json"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"os"
	"reflect"
	"testing"
	"time"
)

// Both policy implementations consume these immutable cases while the migration
// retains Go as the default. Deliberately compare semantic fields, not prose.
func TestPortableRustRoutingContract(t *testing.T) {
	var corpus struct {
		Now   int64 `json:"now"`
		Cases []struct {
			Name     string                    `json:"name"`
			Patch    json.RawMessage           `json:"patch"`
			Request  Request                   `json:"request"`
			Capacity map[string]map[string]any `json:"capacity"`
			Expect   map[string]any            `json:"expect"`
		} `json:"cases"`
	}
	raw, err := os.ReadFile("../../../../contracts/routing-policy-cases.json")
	if err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(raw, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, tc := range corpus.Cases {
		t.Run(tc.Name, func(t *testing.T) {
			matrix, err := Load("", tc.Patch)
			if err != nil {
				t.Fatal(err)
			}
			providers := []any{}
			for provider, c := range tc.Capacity {
				account, ok := c["account"]
				if !ok {
					account = ""
				}
				fresh, ok := c["fresh"]
				if !ok {
					fresh = true
				}
				window := map[string]any{"used_percent": map[string]any{"state": "ok", "value": c["used"]}, "resets_at": corpus.Now + int64(c["resetIn"].(float64)), "window_minutes": c["minutes"], "is_current": true}
				providers = append(providers, map[string]any{"provider": provider, "accounts": []any{map[string]any{"account": account, "is_default": account == "", "fresh": fresh, "windows": map[string]any{"five_hour": window, "seven_day": window, "monthly": map[string]any{"used_percent": map[string]any{"state": "unavailable"}}}}}})
			}
			raw, _ := json.Marshal(map[string]any{"generated_at": corpus.Now, "providers": providers})
			snapshot, err := limits.DecodeReport(raw, time.Unix(corpus.Now, 0))
			if err != nil {
				t.Fatal(err)
			}
			decision := Select(matrix, snapshot, nil, nil, time.Unix(corpus.Now, 0), tc.Request)
			raw, _ = json.Marshal(decision)
			var got map[string]any
			_ = json.Unmarshal(raw, &got)
			var check func(string, map[string]any, map[string]any)
			check = func(path string, want, actual map[string]any) {
				for k, v := range want {
					if obj, ok := v.(map[string]any); ok {
						a, _ := actual[k].(map[string]any)
						check(path+k+".", obj, a)
					} else if !reflect.DeepEqual(v, actual[k]) {
						t.Errorf("%s%s: want %v, got %v; decision %s", path, k, v, actual[k], raw)
					}
				}
			}
			check("", tc.Expect, got)
		})
	}
}
