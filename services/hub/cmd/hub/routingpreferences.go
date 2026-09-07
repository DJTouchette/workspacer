package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

func routingPreferencesTrusted(method string, c bus.CallerIdentity) error {
	if !c.AuthenticatedHost || !c.IsTrusted() || c.Scope != "operator" {
		return fmt.Errorf("%s requires authenticated host authority and operator tier; peer editing is unavailable", method)
	}
	return nil
}
func routingPreferencesGet(s *routing.Service) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		var req struct{}
		if err := routing.DecodePreferences(raw, &req); err != nil {
			return nil, err
		}
		v := s.Preferences()
		v.Configurable = v.Configurable && routingPreferencesTrusted("routing.preferences.get", c) == nil
		return v, nil
	}
}
func routingPreferencesValidate(s *routing.Service) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := routingPreferencesTrusted("routing.preferences.validate", c); err != nil {
			return nil, err
		}
		var req routing.PreferencesRequest
		if err := routing.DecodePreferences(raw, &req); err != nil {
			return nil, err
		}
		return s.UpdatePreferences(req, "validate")
	}
}
func routingPreferencesSave(s *routing.Service) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := routingPreferencesTrusted("routing.preferences.save", c); err != nil {
			return nil, err
		}
		var req routing.PreferencesRequest
		if err := routing.DecodePreferences(raw, &req); err != nil {
			return nil, err
		}
		return s.UpdatePreferences(req, "save")
	}
}
func routingPreferencesReset(s *routing.Service) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := routingPreferencesTrusted("routing.preferences.reset", c); err != nil {
			return nil, err
		}
		var req struct {
			BaseRevision string `json:"baseRevision"`
		}
		if err := routing.DecodePreferences(raw, &req); err != nil {
			return nil, err
		}
		return s.UpdatePreferences(routing.PreferencesRequest{BaseRevision: req.BaseRevision}, "reset")
	}
}

type routingPreviewView struct {
	FellOverFrom   *routing.Assignment `json:"fellOverFrom,omitempty"`
	Role           string              `json:"role"`
	Profile        string              `json:"profile"`
	Provider       string              `json:"provider"`
	Model          string              `json:"model"`
	Effort         string              `json:"effort"`
	Capability     string              `json:"capability"`
	BaseCapability string              `json:"baseCapability"`
	Fresh          bool                `json:"fresh"`
	Eligible       bool                `json:"eligible"`
	Capped         bool                `json:"capped"`
	Mode           routing.Mode        `json:"mode"`
	Reason         []string            `json:"reason"`
	EffortStep     *routing.EffortStep `json:"effortStep,omitempty"`
	ObservedAt     int64               `json:"observedAt"`
	UsageState     string              `json:"usageState"`
}

func routingPreview(s *routing.Service, usage *usageWatcher, avail availabilitySource) bus.LocalIdentHandler {
	return func(_ bus.CallerIdentity, raw json.RawMessage) (any, error) {
		var req routing.Request
		if err := routing.DecodePreferences(raw, &req); err != nil {
			return nil, err
		}
		if strings.TrimSpace(req.Role) == "" {
			return nil, fmt.Errorf("role is required")
		}
		if canonical, ok := bus.CanonicalizeRoot(req.Cwd); ok {
			req.CanonicalCwd = canonical
		}
		s.ReloadIfChanged()
		ctx, cancel := context.WithTimeout(context.Background(), usageDecisionWait)
		defer cancel()
		snap, snapErr := usage.LatestWithin(ctx, usageDecisionMaxAge)
		var availability routing.ProviderAvailability
		if avail != nil {
			availability = avail.Availability()
		}
		now := time.Now()
		d := routing.Select(s.Matrix(), snap, snapErr, availability, now, req)
		// Ceiling reasons embed the trusted mapping key and cwd. Remove those exact
		// sentences rather than returning the security-bearing Decision structure.
		reasons := []string{}
		for _, r := range d.Reason {
			private := false
			if d.Ceiling != nil {
				for _, secret := range d.Ceiling.Because {
					if r == secret {
						private = true
					}
				}
			}
			if !private {
				reasons = append(reasons, r)
			}
		}
		capped := d.Ceiling != nil && (d.Ceiling.CapabilityRefused || d.Ceiling.ToolScopeRefused || d.Ceiling.Denied)
		if capped {
			reasons = append(reasons, "Project safety limits constrained this route")
		}
		state := "live-at-time"
		if snapErr != nil {
			state = "unknown"
		}
		return routingPreviewView{FellOverFrom: d.FellOverFrom, Role: d.Role, Profile: d.Profile, Provider: d.Provider, Model: d.Model, Effort: d.Effort, Capability: d.Capability, BaseCapability: d.BaseCapability, Fresh: d.Fresh, Eligible: d.Eligible, Capped: capped, Mode: d.Mode, Reason: reasons, EffortStep: d.EffortStep, ObservedAt: now.UnixMilli(), UsageState: state}, nil
	}
}
