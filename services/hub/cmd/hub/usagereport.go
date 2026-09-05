package main

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/routing"
	"github.com/djtouchette/workspacer-hub/internal/usageprefs"
)

const usageReportMaxAge = time.Minute
const usageReportValidity = time.Minute

// usageReport only reads the existing usage sampler, the installed matrix and
// the hub's own pacing-schedule preference. It cannot select routing, refresh
// availability, publish, log, or spawn.
//
// The prefs store is a THIRD piece of hub-side state, on the same footing as
// the matrix: the caller supplies nothing, and the no-parameter contract below
// is unchanged. A nil store is "no preference", which reproduces the matrix's
// own answer exactly.
func usageReport(svc *routing.Service, usage *usageWatcher, prefs *usageprefs.Store) bus.LocalIdentHandler {
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
		return snap.UsageReport(time.Now(), prefs.Apply(svc.Matrix().PaceConfig()), usageReportValidity), nil
	}
}
