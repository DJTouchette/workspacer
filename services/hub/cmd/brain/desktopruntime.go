package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"time"
)

func (r *registry) desktopRuntimeStatus(ctx context.Context) (json.RawMessage, error) {
	state := map[string]string{"hub": "ready", "claudemon": "failed", "facade": "unknown"}
	probeCtx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	if _, err := r.cm.getRaw(probeCtx, "/health"); err == nil {
		state["claudemon"] = "ready"
	}
	if r.mcpFacadeURL != "" {
		state["facade"] = "failed"
		if u, err := url.Parse(r.mcpFacadeURL); err == nil {
			u.Path = "/health"
			u.RawQuery = ""
			u.Fragment = ""
			if req, err := http.NewRequestWithContext(probeCtx, http.MethodGet, u.String(), nil); err == nil {
				if res, err := r.cm.http.Do(req); err == nil {
					res.Body.Close()
					if res.StatusCode == http.StatusOK {
						state["facade"] = "ready"
					}
				}
			}
		}
	}
	return json.Marshal(state)
}
