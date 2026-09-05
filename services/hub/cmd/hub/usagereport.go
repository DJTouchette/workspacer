package main

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

const usageReportMaxAge = time.Minute
const usageReportValidity = time.Minute

// usageReport only reads the existing usage sampler and installed matrix.
// It cannot select routing, refresh availability, publish, log, or spawn.
func usageReport(svc *routing.Service, usage *usageWatcher) bus.LocalIdentHandler {
	return func(_ bus.CallerIdentity, params json.RawMessage) (any, error) {
		var args map[string]json.RawMessage
		if len(params) > 0 {
			if err := json.Unmarshal(params, &args); err != nil {
				return nil, fmt.Errorf("usage.report: no parameters accepted")
			}
		}
		if len(args) != 0 {
			return nil, fmt.Errorf("usage.report: no parameters accepted")
		}
		ctx, cancel := context.WithTimeout(context.Background(), usageDecisionWait)
		defer cancel()
		snap, err := usage.LatestWithin(ctx, usageReportMaxAge)
		if err != nil {
			return nil, fmt.Errorf("usage.report unavailable: %w", err)
		}
		return snap.UsageReport(time.Now(), svc.Matrix().PaceConfig(), usageReportValidity), nil
	}
}
