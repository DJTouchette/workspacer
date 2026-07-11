package main

// providers.* — managed-provider discovery for the headless brain, the web/MCP
// counterpart of the desktop's PROVIDER_LIST_MODELS / PROVIDER_CHECK_ALL IPC
// handlers. A faithful Go port of agentProviders.ts: the same PATH probing and
// config binary-override honoring, so a web client's Spawn dialog sees the same
// model catalog and per-provider detection dots the desktop does. The brain runs
// on the same host as the desktop and claudemon, so its PATH view is the real one.

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// managedProviders is the detection set — matches agentProviders.ts's list
// (Claude included so the dialog shows its dot too).
var managedProviders = []string{"claude", "codex", "opencode", "pi"}

// binNames returns the candidate binary names for a provider, platform-aware
// (mirrors agentProviders.binNames). The binary name matches the provider id.
func binNames(base string) []string {
	if runtime.GOOS == "windows" {
		return []string{base + ".cmd", base + ".exe", base}
	}
	return []string{base}
}

// findOnPath returns the first existing absolute path for any of names across
// $PATH, or "" when none is found (mirrors agentProviders.findOnPath).
func findOnPath(names []string) string {
	for _, dir := range filepath.SplitList(os.Getenv("PATH")) {
		if dir == "" {
			continue
		}
		for _, name := range names {
			full := filepath.Join(dir, name)
			if st, err := os.Stat(full); err == nil && !st.IsDir() {
				return full
			}
		}
	}
	return ""
}

// resolveAgentBinary picks the launcher path for a provider: a non-empty config
// override wins; else a fresh PATH search; else the bare command name so a
// just-installed CLI still works (mirrors agentProviders.resolveAgentBinary).
func resolveAgentBinary(provider, customBin string) string {
	if b := strings.TrimSpace(customBin); b != "" {
		return b
	}
	if p := findOnPath(binNames(provider)); p != "" {
		return p
	}
	return provider
}

// providerStatus mirrors agentProviders.ProviderStatus. ResolvedPath is a pointer
// so a missing binary serializes as JSON null (the shape the renderer expects:
// `resolvedPath: string | null`).
type providerStatus struct {
	Provider     string  `json:"provider"`
	Found        bool    `json:"found"`
	ResolvedPath *string `json:"resolvedPath"`
	CustomBin    string  `json:"customBin"`
}

// checkAllProviders reports detection status for every provider, honoring the
// user's per-provider binary overrides from config (mirrors
// agentProviders.checkAllProviders).
func checkAllProviders(binaries map[string]string) []providerStatus {
	out := make([]providerStatus, 0, len(managedProviders))
	for _, provider := range managedProviders {
		customBin := strings.TrimSpace(binaries[provider])
		if customBin != "" {
			found := false
			if st, err := os.Stat(customBin); err == nil && !st.IsDir() {
				found = true
			}
			var rp *string
			if found {
				cb := customBin
				rp = &cb
			}
			out = append(out, providerStatus{Provider: provider, Found: found, ResolvedPath: rp, CustomBin: customBin})
			continue
		}
		resolved := findOnPath(binNames(provider))
		var rp *string
		if resolved != "" {
			r := resolved
			rp = &r
		}
		out = append(out, providerStatus{Provider: provider, Found: resolved != "", ResolvedPath: rp, CustomBin: ""})
	}
	return out
}

// providerBinaries reads config.agents.binaries as a plain map[string]string,
// tolerating a missing/oddly-typed section (headless config has no agents key by
// default). Used to seed both list-model bin resolution and detection.
func (r *registry) providerBinaries() map[string]string {
	out := map[string]string{}
	cfg := r.cfg.get()
	agents, _ := cfg["agents"].(map[string]any)
	if agents == nil {
		return out
	}
	bins, _ := agents["binaries"].(map[string]any)
	for k, v := range bins {
		if s, ok := v.(string); ok {
			out[k] = s
		}
	}
	return out
}

// resolveSpawnBin picks the launcher for a spawn: the user's config override
// (agents.binaries) wins, then — for claude, keeping parity with the PTY path's
// buildArgv — the WKS_CLAUDE_BIN escape hatch, then a PATH probe, then the bare
// provider name.
func (r *registry) resolveSpawnBin(provider string) string {
	custom := r.providerBinaries()[provider]
	if provider == "claude" && strings.TrimSpace(custom) == "" {
		custom = os.Getenv("WKS_CLAUDE_BIN")
	}
	return resolveAgentBinary(provider, custom)
}

// providersListModels relays to claudemon's live model query, resolving the
// launcher the same way spawning does. It soft-fails to an empty array (matching
// claudemonSessionClient.listProviderModels) so the Spawn dialog falls back to
// free-text entry when the provider CLI is missing or errors — never a hard error.
func (r *registry) providersListModels(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Provider string `json:"provider"`
		Cwd      string `json:"cwd"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.Provider != "codex" && p.Provider != "opencode" && p.Provider != "pi" {
		return nil, fmt.Errorf("providers.listModels requires { provider: 'codex'|'opencode'|'pi' }")
	}
	bin := resolveAgentBinary(p.Provider, r.providerBinaries()[p.Provider])
	body, err := r.cm.providerModels(ctx, p.Provider, p.Cwd, bin)
	if err != nil {
		return jsonResult([]any{})
	}
	var parsed struct {
		Models []struct {
			ID      string `json:"id"`
			Label   string `json:"label"`
			Default bool   `json:"default"`
		} `json:"models"`
	}
	if err := json.Unmarshal(body, &parsed); err != nil {
		return jsonResult([]any{})
	}
	out := make([]map[string]any, 0, len(parsed.Models))
	for _, m := range parsed.Models {
		out = append(out, map[string]any{"id": m.ID, "label": m.Label, "default": m.Default})
	}
	return jsonResult(out)
}

// providersCheckAll returns per-provider detection status, honoring config binary
// overrides. Real (not a stub): the brain shares the host with the desktop, so
// its PATH probe is the same one the desktop's IPC handler runs.
func (r *registry) providersCheckAll() (json.RawMessage, error) {
	return jsonResult(checkAllProviders(r.providerBinaries()))
}
