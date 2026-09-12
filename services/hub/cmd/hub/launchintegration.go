package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

// This callback grants no ambient worker access to plugin configuration or
// credentials. Its authority comes from a still-active owner spawn on the same
// provider connection, verified before AND after the sidecar call.
func launchPreparation(srv *bus.Server, list func() []plugin.Manifest, call func(context.Context, string, any) (json.RawMessage, error)) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		var p struct {
			CallID   string `json:"callId"`
			PluginID string `json:"pluginId"`
			Op       string `json:"op"`
			Context  struct {
				Version  int    `json:"version"`
				Agent    string `json:"agent"`
				Cwd      string `json:"cwd"`
				Model    string `json:"model,omitempty"`
				Resume   bool   `json:"resume"`
				Provider *struct {
					ID      string `json:"id"`
					BaseURL string `json:"baseUrl,omitempty"`
				} `json:"provider,omitempty"`
			} `json:"context"`
		}
		if err := json.Unmarshal(raw, &p); err != nil {
			return nil, err
		}
		if err := authorizeLaunchPreparation("plugins.prepareLaunch", srv, c, p.CallID, p.PluginID); err != nil {
			return nil, err
		}
		var selected *plugin.Manifest
		for _, item := range list() {
			if item.ID == p.PluginID && !item.Disabled {
				copy := item
				selected = &copy
				break
			}
		}
		if selected == nil || selected.LaunchIntegration == nil {
			return nil, fmt.Errorf("selected launch integration is unavailable")
		}
		contribution := selected.LaunchIntegration
		if contribution.Version != 1 || !strings.HasPrefix(contribution.PrepareMethod, p.PluginID+".") {
			return nil, fmt.Errorf("invalid launch integration contribution")
		}
		provided := false
		for _, method := range selected.Provides {
			if method == contribution.PrepareMethod {
				provided = true
			}
		}
		if !provided {
			return nil, fmt.Errorf("launch integration method is not declared")
		}
		if p.Op == "describe" {
			return []any{map[string]any{"id": selected.ID, "launchIntegration": contribution, "provides": selected.Provides, "disabled": false}}, nil
		}
		if p.Op != "prepare" || p.Context.Version != 1 || p.Context.Cwd == "" {
			return nil, fmt.Errorf("invalid launch preparation context")
		}
		supported := false
		for _, agent := range contribution.Agents {
			if agent == p.Context.Agent {
				supported = true
			}
		}
		if !supported {
			return nil, fmt.Errorf("integration does not support this agent")
		}
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		defer cancel()
		result, err := call(ctx, contribution.PrepareMethod, p.Context)
		if err != nil {
			return nil, err
		}
		if err := authorizeLaunchPreparation("plugins.prepareLaunch", srv, c, p.CallID, p.PluginID); err != nil {
			return nil, err
		}
		return result, nil
	}
}

func authorizeLaunchPreparation(method string, srv *bus.Server, c bus.CallerIdentity, callID, pluginID string) error {
	if method != "plugins.prepareLaunch" {
		return fmt.Errorf("invalid launch callback capability")
	}
	return srv.AuthorizeLaunchPreparation(c, callID, pluginID)
}
