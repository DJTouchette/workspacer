package limits

import (
	"encoding/json"
	"math"
	"os"
	"testing"
	"time"
)

func TestPortableRustPacingContract(t *testing.T) {
	var corpus struct {
		Now    int64            `json:"now"`
		Config map[string]any   `json:"config"`
		Cases  []map[string]any `json:"cases"`
	}
	raw, e := os.ReadFile("../../../../contracts/usage-pacing-cases.json")
	if e != nil {
		t.Fatal(e)
	}
	if e = json.Unmarshal(raw, &corpus); e != nil {
		t.Fatal(e)
	}
	var merge func(map[string]any, map[string]any)
	merge = func(a, b map[string]any) {
		for k, v := range b {
			if object, ok := v.(map[string]any); ok {
				dst, _ := a[k].(map[string]any)
				if dst == nil {
					dst = map[string]any{}
					a[k] = dst
				}
				merge(dst, object)
			} else {
				a[k] = v
			}
		}
	}
	for _, c := range corpus.Cases {
		t.Run(c["name"].(string), func(t *testing.T) {
			encoded, _ := json.Marshal(corpus.Config)
			var cfg map[string]any
			_ = json.Unmarshal(encoded, &cfg)
			if patch, ok := c["patch"].(map[string]any); ok {
				merge(cfg, patch)
			}
			now := corpus.Now
			if at, ok := c["now"].(float64); ok {
				now = int64(at)
			}
			name := "five_hour"
			if n, ok := c["window"].(string); ok {
				name = n
			}
			w := &WireWindow{UsedPercent: &Measured{State: MeasuredOk}}
			if state, ok := c["measurement"].(string); ok {
				w.UsedPercent.State = MeasuredState(state)
			}
			if used, ok := c["used"].(float64); ok {
				w.UsedPercent.Value = &used
			}
			if reset, ok := c["resetIn"].(float64); ok {
				r := now + int64(reset)
				w.ResetsAt = &r
			}
			if minutes, ok := c["minutes"].(float64); ok {
				m := int64(minutes)
				w.WindowMinutes = &m
			}
			bootstrap := cfg["bootstrap"].(map[string]any)
			week := cfg["seven_day"].(map[string]any)
			loc, _ := time.LoadLocation(week["timezone"].(string))
			p := PaceConfig{Enabled: cfg["enabled"].(bool), ConserveAtRatio: cfg["conserve_at_ratio"].(float64), BlockSpendDownAtRatio: cfg["block_spend_down_at_ratio"].(float64), MinElapsedPct: bootstrap["min_elapsed_pct"].(float64), ExpectedOffsetPct: bootstrap["expected_offset_pct"].(float64), Curve: week["curve"].(string), Location: loc, WeekendWeight: week["weekend_weight"].(float64), WeekendPolicy: week["weekend"].(string), WeekendReservePct: week["weekend_reserve_pct"].(float64)}
			bucket := paceBucket("codex", name, w, time.Unix(now, 0))
			if fresh, ok := c["fresh"].(bool); ok {
				bucket.Fresh = &fresh
			}
			got := PaceFor(bucket, p)
			expect := c["expect"].(map[string]any)
			if string(got.State) != expect["state"] || got.Known != expect["known"] {
				t.Fatalf("pace=%+v want=%v", got, expect)
			}
			if ratio, ok := expect["ratio"].(float64); ok && math.Abs(got.Ratio-ratio) > 1e-10 {
				t.Errorf("ratio=%v want=%v", got.Ratio, ratio)
			}
			if curve, ok := expect["curve"].(string); ok && got.Curve != curve {
				t.Errorf("curve=%s want=%s", got.Curve, curve)
			}
			if health, ok := c["health"].(string); ok && string(BucketHealth(bucket, testBands).Health) != health {
				t.Errorf("health=%s want=%s", BucketHealth(bucket, testBands).Health, health)
			}
		})
	}
}
