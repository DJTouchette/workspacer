---
title: Pace-aware routing: the Anthropic window lengths are asserted by claudemon, not reported by the endpoint
date: 2026-09-01
confidence: high
suggested_doc: limit-aware-routing
promoted: false
---

# Pace-aware routing: the Anthropic window lengths are asserted by claudemon, not reported by the endpoint

## Observation
Anthropic's /api/oauth/usage returns a utilization and a resets_at per window and NEVER a window length, so before this change every Claude window went out of GET /usage/report with window_minutes: null. That single missing term is why no consumer could tell "60% used with half the window left" from "60% used with an hour left". services/claudemon/src/session/usage_report.rs now stamps ANTHROPIC_FIVE_HOUR_MINUTES=300 / ANTHROPIC_SEVEN_DAY_MINUTES=10080 on the OAuth arm only (they are facts about the product, asserted by the window NAMES, not derived from the response), and deliberately leaves the monthly overage window with no length because a calendar month is not a fixed number of minutes. The hub consumes it in internal/limits/pace.go: elapsed = (length - time_to_reset)/length, ratio = used/expected, all read through Reading so a rolled-over window cannot be paced. thresholds.pacing in routing.yaml holds every number; routing/pacing.go is the ONLY mapping from the file to limits.PaceConfig, and thresholds.pacing.enabled:false reproduces the pre-pacing answer down to the absent pace fields on the answer and on the routing.decision event.</observation>
<parameter name="impact">If the claudemon-side constants are ever dropped or the OAuth arm stops stamping them, every Claude pace verdict silently degrades to UNKNOWN — which conserves nothing and blocks nothing, so routing keeps working and the feature quietly stops existing. The routing harness's "a current claude window is PACED at all" check is what catches that; the Go unit tests cannot, because they build their own wire fixtures.</impact>
<parameter name="recommendation">When touching usage_report.rs's claude_report or the pacing block, run `make test-routing-harness` (ROUTING_HARNESS_REQUIRE_ROUTING=1) as well as the Go tests — the end-to-end arrival of window_minutes is only proven there. Note also that keep-warm (apps/desktop keepWarmLogic.ts) types window_minutes but never reads it, so adding lengths to Claude windows had no desktop-side effect.</recommendation>
<parameter name="related_paths">["services/claudemon/src/session/usage_report.rs", "services/hub/internal/limits/pace.go", "services/hub/internal/routing/pacing.go", "services/hub/internal/routing/routing.default.yaml", "services/hub/scripts/routing-limit-harness.mjs"]
