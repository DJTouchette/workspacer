package main

import (
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/federation"
)

type peerConfig struct {
	mu         sync.Mutex
	path       string
	controller *federation.Controller
	fixed      []federation.Peer
}

func peerConfigTrusted(method string, c bus.CallerIdentity) error {
	if !c.AuthenticatedHost || !c.IsTrusted() || c.Scope != "operator" {
		return fmt.Errorf("%s requires the server owner", method)
	}
	return nil
}
func peerConfigRead(p *peerConfig) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, _ json.RawMessage) (any, error) {
		if err := peerConfigTrusted("federation.peersConfig", c); err != nil {
			return nil, err
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		if p.path == "" {
			return nil, fmt.Errorf("peer configuration is disabled by the server launcher")
		}
		peers, err := federation.LoadPeersFile(p.path)
		if err != nil {
			return nil, err
		}
		rows := []map[string]any{}
		for _, peer := range peers {
			rows = append(rows, map[string]any{"name": peer.Name, "url": peer.URL, "hasToken": peer.Token != "", "dispatch": peer.Dispatch})
		}
		return rows, nil
	}
}

var peerConfigName = regexp.MustCompile(`^[A-Za-z0-9_-]+$`)

func peerConfigSave(p *peerConfig) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := peerConfigTrusted("federation.savePeersConfig", c); err != nil {
			return nil, err
		}
		var request struct {
			Peers []struct {
				Name     string  `json:"name"`
				URL      string  `json:"url"`
				Token    *string `json:"token"`
				Dispatch bool    `json:"dispatch"`
			} `json:"peers"`
		}
		if err := json.Unmarshal(raw, &request); err != nil {
			return nil, err
		}
		if request.Peers == nil || len(request.Peers) > 64 {
			return nil, fmt.Errorf("peers must be an array of at most 64 entries")
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		if p.path == "" {
			return nil, fmt.Errorf("peer configuration is disabled by the server launcher")
		}
		existing, err := federation.LoadPeersFile(p.path)
		if err != nil {
			return nil, err
		}
		stored := map[string]string{}
		for _, peer := range existing {
			stored[peer.Name] = peer.Token
		}
		peers := []federation.Peer{}
		rows := []map[string]any{}
		seen := map[string]bool{}
		for _, entry := range request.Peers {
			name, address := strings.TrimSpace(entry.Name), strings.TrimSpace(entry.URL)
			if !peerConfigName.MatchString(name) || len(name) > 128 {
				return nil, fmt.Errorf("peer names use letters, digits, - or _")
			}
			parsed, err := url.Parse(address)
			if err != nil || (parsed.Scheme != "ws" && parsed.Scheme != "wss") || parsed.Host == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
				return nil, fmt.Errorf("peer URL must be a credential-free ws:// or wss:// address")
			}
			if seen[name] {
				return nil, fmt.Errorf("duplicate peer name %q", name)
			}
			seen[name] = true
			token := stored[name]
			if entry.Token != nil {
				token = strings.TrimSpace(*entry.Token)
			}
			if len(token) > 4096 {
				return nil, fmt.Errorf("peer token is too long")
			}
			peers = append(peers, federation.Peer{Name: name, URL: address, Token: token, Dispatch: entry.Dispatch})
			rows = append(rows, map[string]any{"name": name, "url": address, "token": token, "dispatch": entry.Dispatch})
		}
		all := append(append([]federation.Peer{}, peers...), p.fixed...)
		// Validate the full set before persisting, including launcher-defined peers.
		if _, err := federation.New(nil, all); err != nil {
			return nil, err
		}
		data, err := json.MarshalIndent(rows, "", "  ")
		if err != nil {
			return nil, err
		}
		if err := os.MkdirAll(filepath.Dir(p.path), 0700); err != nil {
			return nil, err
		}
		temporary, err := os.CreateTemp(filepath.Dir(p.path), ".peers-*")
		if err != nil {
			return nil, err
		}
		defer os.Remove(temporary.Name())
		if err = temporary.Chmod(0600); err == nil {
			_, err = temporary.Write(append(data, '\n'))
		}
		closeErr := temporary.Close()
		if err != nil {
			return nil, err
		}
		if closeErr != nil {
			return nil, closeErr
		}
		if err := os.Rename(temporary.Name(), p.path); err != nil {
			return nil, err
		}
		if err := p.controller.Replace(all); err != nil {
			return nil, err
		}
		return map[string]any{"ok": true}, nil
	}
}
