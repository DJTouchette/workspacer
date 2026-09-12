package main

import (
	"encoding/json"
	"fmt"
	"strings"
	"sync"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

// Pairing administration uses the hub's configured token store, not a browser
// credential or the worker's separate store. Infrastructure tokens are excluded.
type remotePairings struct {
	path string
	mu   sync.Mutex
}

func remotePairingTrusted(method string, c bus.CallerIdentity) error {
	if !c.AuthenticatedHost || !c.IsTrusted() || c.Scope != "operator" {
		return fmt.Errorf("%s requires the server owner's pairing token", method)
	}
	return nil
}
func isRemotePairing(r authtoken.Record) bool {
	return (r.Scope == authtoken.ScopeView || r.Scope == authtoken.ScopeTriage || r.Scope == authtoken.ScopeOperator) &&
		strings.HasPrefix(r.Label, "Remote Control: ") && r.Role == "" && !r.YoloAllowed && len(r.ProfilesAllowed) == 0 && len(r.Plugins) == 0 && len(r.Provides) == 0
}
func remotePairingInfo(p *remotePairings) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, _ json.RawMessage) (any, error) {
		return map[string]any{"scope": c.Scope, "canManageTokens": p.path != "" && remotePairingTrusted("remote.pairingInfo", c) == nil}, nil
	}
}
func remoteTokensList(p *remotePairings) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, _ json.RawMessage) (any, error) {
		if err := remotePairingTrusted("remote.tokensList", c); err != nil {
			return nil, err
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		if p.path == "" {
			return nil, fmt.Errorf("pairing token store is disabled")
		}
		recs, err := authtoken.Load(p.path)
		if err != nil {
			return nil, err
		}
		out := []authtoken.Record{}
		for _, r := range recs {
			if isRemotePairing(r) {
				out = append(out, r)
			}
		}
		return out, nil
	}
}
func remoteTokenGetOrCreate(p *remotePairings) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := remotePairingTrusted("remote.tokenGetOrCreate", c); err != nil {
			return nil, err
		}
		var req struct {
			Scope string `json:"scope"`
		}
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		scope := authtoken.Scope(req.Scope)
		if scope != authtoken.ScopeView && scope != authtoken.ScopeTriage && scope != authtoken.ScopeOperator {
			return nil, fmt.Errorf("pairing scope must be view, triage or operator")
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		if p.path == "" {
			return nil, fmt.Errorf("pairing token store is disabled")
		}
		recs, err := authtoken.Load(p.path)
		if err != nil {
			return nil, err
		}
		label := "Remote Control: " + req.Scope
		for _, r := range recs {
			if r.Scope == scope && r.Label == label && isRemotePairing(r) {
				return r, nil
			}
		}
		return authtoken.Mint(p.path, scope, label)
	}
}
func remoteTokenRevoke(p *remotePairings) bus.LocalIdentHandler {
	return func(c bus.CallerIdentity, raw json.RawMessage) (any, error) {
		if err := remotePairingTrusted("remote.tokenRevoke", c); err != nil {
			return nil, err
		}
		var req struct {
			Token string `json:"token"`
		}
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		p.mu.Lock()
		defer p.mu.Unlock()
		if p.path == "" {
			return nil, fmt.Errorf("pairing token store is disabled")
		}
		recs, err := authtoken.Load(p.path)
		if err != nil {
			return nil, err
		}
		for i, r := range recs {
			if r.Token == req.Token && isRemotePairing(r) {
				if err := authtoken.Save(p.path, append(recs[:i], recs[i+1:]...)); err != nil {
					return nil, err
				}
				return r, nil
			}
		}
		return nil, fmt.Errorf("pairing token not found; infrastructure credentials cannot be revoked here")
	}
}
