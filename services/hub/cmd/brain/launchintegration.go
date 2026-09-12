package main

import (
	"context"
	"encoding/json"
	"fmt"
)

func (r *registry) prepareIntegration(ctx context.Context, p spawnParams, agent, cwd, bin string, env map[string]string, args []string) (map[string]string, []string, error) {
	if !p.LaunchIntegrationGranted {
		return nil, nil, fmt.Errorf("launch integration was not authorized by the hub owner")
	}
	if env == nil {
		env = map[string]string{}
	}
	if args == nil {
		args = []string{}
	}
	payload, _ := json.Marshal(map[string]any{"id": p.LaunchIntegrationID, "launchContext": map[string]any{"agent": agent, "cwd": cwd, "model": p.Model, "resume": p.ResumeSessionID != ""}, "base": map[string]any{"env": env, "args": args, "bin": bin}})
	result, err := r.desktopInternalCall(ctx, "internal.prepareIntegration", payload)
	if err != nil {
		return nil, nil, err
	}
	var patch struct {
		Env  map[string]string `json:"env"`
		Args []string          `json:"args"`
	}
	if err := json.Unmarshal(result, &patch); err != nil {
		return nil, nil, err
	}
	return patch.Env, patch.Args, nil
}
