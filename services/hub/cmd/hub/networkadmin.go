package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"os/user"
	"runtime"
	"strconv"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/bus"
)

type networkAdmin struct {
	socket    string
	tokenFile string
	port      int
}

func networkTrusted(method string, c bus.CallerIdentity) error {
	if !c.AuthenticatedHost || !c.IsTrusted() || c.Scope != "operator" {
		return fmt.Errorf("%s requires the server owner", method)
	}
	return nil
}
func (p *networkAdmin) broker(ctx context.Context, method, path string, body any) (json.RawMessage, error) {
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "unix", p.socket)
	}}
	defer transport.CloseIdleConnections()
	var data []byte
	if body != nil {
		data, _ = json.Marshal(body)
	}
	request, err := http.NewRequestWithContext(ctx, method, "http://network-admin"+path, bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	credential, err := os.ReadFile(p.tokenFile)
	if err != nil || len(bytes.TrimSpace(credential)) == 0 {
		return nil, fmt.Errorf("private network credential unavailable")
	}
	request.Header.Set("Authorization", "Bearer "+string(bytes.TrimSpace(credential)))
	response, err := (&http.Client{Transport: transport, Timeout: 40 * time.Second}).Do(request)
	if err != nil {
		return nil, fmt.Errorf("server network control unavailable")
	}
	defer response.Body.Close()
	result, err := io.ReadAll(io.LimitReader(response.Body, 32*1024))
	if err != nil {
		return nil, err
	}
	if response.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("server network change was not confirmed (HTTP %d)", response.StatusCode)
	}
	return result, nil
}
func (p *networkAdmin) command(ctx context.Context, args ...string) ([]byte, error) {
	return exec.CommandContext(ctx, "tailscale", args...).Output()
}
func (p *networkAdmin) info(ctx context.Context) (map[string]any, error) {
	if p.socket != "" {
		data, err := p.broker(ctx, "GET", "/status", nil)
		if err != nil {
			return nil, err
		}
		var info map[string]any
		err = json.Unmarshal(data, &info)
		return info, err
	}
	data, err := p.command(ctx, "status", "--json")
	if err != nil {
		return map[string]any{"available": false, "magicName": nil, "serveActive": false, "canServe": false}, nil
	}
	var status struct {
		BackendState string
		Self         struct{ DNSName string }
	}
	if err := json.Unmarshal(data, &status); err != nil {
		return nil, err
	}
	available := status.BackendState == "Running"
	serving, _ := p.command(ctx, "serve", "status", "--json")
	active := bytes.Contains(serving, []byte("127.0.0.1:"+strconv.Itoa(p.port))) || bytes.Contains(serving, []byte("localhost:"+strconv.Itoa(p.port)))
	permitted := runtime.GOOS != "linux" || os.Geteuid() == 0
	if !permitted {
		prefs, _ := p.command(ctx, "debug", "prefs")
		var decoded struct{ OperatorUser string }
		_ = json.Unmarshal(prefs, &decoded)
		if current, err := user.Current(); err == nil {
			permitted = decoded.OperatorUser == current.Username || decoded.OperatorUser == current.Uid
		}
	}
	result := map[string]any{"available": available, "magicName": strings.TrimSuffix(status.Self.DNSName, "."), "serveActive": active, "canServe": available && permitted}
	if !permitted {
		result["hint"] = "Configure the server's Tailscale operator to allow HTTPS sharing changes"
	}
	return result, nil
}
func networkInfo(p *networkAdmin) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, _ json.RawMessage) (any, error) {
		if err := networkTrusted("remote.tailscaleInfo", c); err != nil {
			return nil, err
		}
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		return p.info(ctx)
	}
}
func networkServe(p *networkAdmin) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := networkTrusted("remote.tailscaleServe", c); err != nil {
			return nil, err
		}
		var req struct {
			Enabled *bool `json:"enabled"`
		}
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		if req.Enabled == nil {
			return nil, fmt.Errorf("enabled boolean required")
		}
		return p.setServe(*req.Enabled)
	}
}
func (p *networkAdmin) setServe(enabled bool) (any, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 120*time.Second)
	defer cancel()
	if p.socket != "" {
		return p.broker(ctx, "POST", "/serve", map[string]bool{"enabled": enabled})
	}
	args := []string{"serve", "reset"}
	if enabled {
		args = []string{"serve", "--bg", strconv.Itoa(p.port)}
	}
	if _, err := p.command(ctx, args...); err != nil {
		return nil, fmt.Errorf("Tailscale Serve failed; check the server's Tailscale permissions and HTTPS configuration")
	}
	return map[string]bool{"ok": true}, nil
}
func sharingInfo(p *networkAdmin) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, _ json.RawMessage) (any, error) {
		// Availability is safe to disclose; changing a network listener is not.
		result := map[string]any{"enabled": true, "canToggleSharing": p.socket != "" && networkTrusted("remote.setSharing", c) == nil}
		if p.socket != "" {
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			info, err := p.info(ctx)
			if err != nil {
				result["canToggleSharing"] = false
				result["error"] = err.Error()
			} else {
				result["enabled"] = info["serveActive"]
			}
		}
		return result, nil
	}
}
func setSharing(p *networkAdmin) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := networkTrusted("remote.setSharing", c); err != nil {
			return nil, err
		}
		if p.socket == "" {
			return nil, fmt.Errorf("this server's network listener is managed by its launcher")
		}
		var req struct {
			Enabled *bool `json:"enabled"`
		}
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		if req.Enabled == nil {
			return nil, fmt.Errorf("enabled boolean required")
		}
		if _, err := p.setServe(*req.Enabled); err != nil {
			return nil, err
		}
		return map[string]any{"enabled": *req.Enabled, "canToggleSharing": true}, nil
	}
}
