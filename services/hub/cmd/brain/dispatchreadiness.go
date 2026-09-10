package main

// Remote readiness is measured on the execution host. CLI login probes return
// only a tri-state result; their bounded output is never logged or returned.
// Repository choices come from this host config and active sessions, never a
// desktop path translation or an automatic clone.

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// dispatchProvider is one harness's readiness on this node.
type dispatchProvider struct {
	Provider string `json:"provider"`
	Found    bool   `json:"found"`
	// Authenticated is nil when this node cannot know (see the header). A
	// pointer, so it serializes as JSON null rather than collapsing to false.
	Authenticated *bool `json:"authenticated"`
	// Note is the one sentence a manager can act on. Always populated, because
	// "codex: installed, not signed in" and "copilot: installed, login state
	// unreadable" call for different decisions and a bare tri-state does not
	// say which is which.
	Note string `json:"note"`
}

func boolPtr(v bool) *bool { return &v }

// dispatchProviderReadiness reports every managed provider's usability here.
func (r *registry) dispatchProviderReadiness(ctx context.Context) []dispatchProvider {
	out := make([]dispatchProvider, 0, len(managedProviders))
	for _, st := range checkAllProviders(r.providerBinaries()) {
		p := dispatchProvider{Provider: st.Provider, Found: st.Found}
		if !st.Found {
			p.Note = "not installed on this machine"
			p.Authenticated = boolPtr(false)
			out = append(out, p)
			continue
		}
		if st.ResolvedPath != nil {
			p.Authenticated = probeProviderLogin(ctx, *st.ResolvedPath, st.Provider)
		}
		switch {
		case p.Authenticated == nil:
			p.Note = "installed; this host could not confirm a usable login"
		case *p.Authenticated:
			p.Note = "installed; provider login status confirmed on this host"
		default:
			p.Note = "installed but not logged in on this host"
		}
		out = append(out, p)
	}
	return out
}

// Probe the actual CLI login state, not a stale account label or file presence.
// Output is bounded, parsed here and never logged or returned.
func probeProviderLogin(ctx context.Context, binary, provider string) *bool {
	var args []string
	switch provider {
	case "claude":
		args = []string{"auth", "status", "--json"}
	case "codex":
		args = []string{"login", "status"}
	default:
		return nil
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, args...)
	output := &cappedWriter{limit: 64 * 1024}
	cmd.Stdout, cmd.Stderr = output, output
	err := cmd.Run()
	if ctx.Err() != nil || output.over {
		return nil
	}
	return providerLoginFromOutput(provider, output.buf.Bytes(), err == nil)
}

func providerLoginFromOutput(provider string, output []byte, success bool) *bool {
	if provider == "claude" {
		var status struct {
			LoggedIn *bool `json:"loggedIn"`
		}
		if json.Unmarshal(output, &status) == nil {
			if status.LoggedIn != nil && *status.LoggedIn && !success {
				return nil
			}
			return status.LoggedIn
		}
		return nil
	}
	if strings.Contains(strings.ToLower(string(output)), "not logged in") {
		return boolPtr(false)
	}
	if success && strings.Contains(strings.ToLower(string(output)), "logged in") {
		return boolPtr(true)
	}
	return nil
}

// dispatchCwd is one directory on THIS machine a dispatched worker may be
// pointed at.
type dispatchCwd struct {
	Path string `json:"path"`
	// Source says where the path came from — "project" (configured here) or
	// "active" (an agent is working in it right now). A manager choosing between
	// two candidates deserves to know which one this machine actually uses.
	Source string `json:"source"`
	// Git is true when the directory is a git working tree, which is what makes
	// it a candidate for ship work rather than only for reading.
	Git bool `json:"git"`
}

// dispatchCwdChoices lists the real remote directories a dispatch may name.
//
// Deliberately NOT a filesystem walk and NOT the home tree: those are a browse
// surface, and this is a menu. The two sources are the ones this node has
// already committed to — its configured projects and the cwds its live agents
// hold — so every entry is a directory some part of this machine is already
// using, and the list stays short enough to put in a tool result.
func (r *registry) dispatchCwdChoices(ctx context.Context) []dispatchCwd {
	seen := map[string]string{}
	if cfg := r.cfg.get(); cfg != nil {
		if projects, ok := cfg["projects"].(map[string]any); ok {
			for path := range projects {
				if p := strings.TrimSpace(path); p != "" {
					seen[filepath.Clean(p)] = "project"
				}
			}
		}
	}
	for _, cwd := range r.agentCwds(ctx) {
		p := filepath.Clean(strings.TrimSpace(cwd))
		if p == "" || p == "." {
			continue
		}
		if _, ok := seen[p]; !ok {
			seen[p] = "active"
		}
	}
	out := make([]dispatchCwd, 0, len(seen))
	for path, source := range seen {
		st, err := os.Stat(path)
		if err != nil || !st.IsDir() {
			// A configured project whose directory is gone is not a choice. Drop
			// it rather than offering a cwd whose spawn would fail here.
			continue
		}
		out = append(out, dispatchCwd{Path: path, Source: source, Git: isGitWorkTree(path)})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Path < out[j].Path })
	return out
}

// isGitWorkTree reports whether a directory is the root of a git checkout —
// `.git` as either a directory or the file a worktree/submodule uses.
func isGitWorkTree(dir string) bool {
	_, err := os.Stat(filepath.Join(dir, ".git"))
	return err == nil
}
