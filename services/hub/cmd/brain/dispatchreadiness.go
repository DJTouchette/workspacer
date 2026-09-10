package main

// THE HONEST READINESS ANSWER for a machine somebody else is about to dispatch
// work onto — the data half of fleet.dispatchCapabilities (remotedispatch.go).
//
// THE RULE THIS FILE EXISTS TO ENFORCE: a dispatching machine may not infer
// anything about this one. Not which harnesses are installed, not which are
// LOGGED IN, and above all not which directories exist. The failure it prevents
// is specific and was observed on a real combined Fly node: Codex is installed
// there and has no login at all, while Claude is signed in — so a manager on a
// desktop where both work would have dispatched a Codex worker that opens a
// session, answers nothing, and ends. That reads exactly like "the first message
// never arrived", and the manager cannot tell the difference.
//
// So every field below is measured HERE:
//
//   - `found` is a PATH probe on this machine, through the same
//     checkAllProviders the desktop's own provider check uses.
//   - `authenticated` is read off this machine's credential file, and it is
//     TRI-STATE: true, false, or ABSENT for a harness that keeps its login
//     somewhere unreadable (copilot uses the OS credential store). Absent means
//     "this node cannot tell", never "probably fine" — the note says which.
//   - `cwds` are real absolute directories on THIS filesystem: the projects this
//     node has configured, plus the directories its live agents are already
//     working in. There is NO path translation anywhere in this feature; a
//     dispatch names one of these verbatim or it names a directory the spawn
//     will reject.
//
// NOTHING HERE READS A SECRET'S VALUE. The credential files are probed for
// presence and, for codex, for two non-secret fields; no token is ever loaded,
// returned, or logged.

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
	if provider == "claude" {
		var status struct {
			LoggedIn *bool `json:"loggedIn"`
		}
		if json.Unmarshal([]byte(output.buf.String()), &status) == nil {
			return status.LoggedIn
		}
		return nil
	}
	if err == nil && strings.Contains(strings.ToLower(output.buf.String()), "logged in") {
		return boolPtr(true)
	}
	if strings.Contains(strings.ToLower(output.buf.String()), "not logged in") {
		return boolPtr(false)
	}
	return nil
}

// claudeConfigRoot is where the Claude CLI keeps its state on this machine:
// CLAUDE_CONFIG_DIR when set, else ~/.claude. TWIN of the resolution
// apps/desktop/src/main/lib/profileAccounts.ts performs for the DEFAULT profile.
func claudeConfigRoot() string {
	if v := strings.TrimSpace(os.Getenv("CLAUDE_CONFIG_DIR")); v != "" {
		return expandTilde(v)
	}
	home := homeDir()
	if home == "" {
		return ""
	}
	return filepath.Join(home, ".claude")
}

// claudeLoginPresent reports whether this machine has a usable Claude login.
//
// Two sources, matching profileAccounts.ts: the OAuth credential file inside
// the config root, and `oauthAccount` in `.claude.json` (whose location differs
// for the default root — the quirk the desktop's claudeAccountSetup owns). An
// API key in the environment counts too: it is how a headless node is most
// often credentialled.
func claudeLoginPresent() bool {
	if strings.TrimSpace(os.Getenv("ANTHROPIC_API_KEY")) != "" {
		return true
	}
	root := claudeConfigRoot()
	if root == "" {
		return false
	}
	if fileExists(filepath.Join(root, ".credentials.json")) {
		return true
	}
	// The default root keeps its json beside the directory, not inside it.
	candidates := []string{filepath.Join(root, ".claude.json")}
	if home := homeDir(); home != "" && filepath.Clean(root) == filepath.Join(home, ".claude") {
		candidates = append(candidates, filepath.Join(home, ".claude.json"))
	}
	for _, c := range candidates {
		var doc struct {
			OAuthAccount map[string]any `json:"oauthAccount"`
		}
		if readJSONFile(c, &doc) && len(doc.OAuthAccount) > 0 {
			return true
		}
	}
	return false
}

// codexLoginPresent reads $CODEX_HOME/auth.json for the same three signals
// codexAccountFromAuthFile checks, and NONE of their values: presence only.
func codexLoginPresent() bool {
	if strings.TrimSpace(os.Getenv("OPENAI_API_KEY")) != "" {
		return true
	}
	root := strings.TrimSpace(os.Getenv("CODEX_HOME"))
	if root == "" {
		home := homeDir()
		if home == "" {
			return false
		}
		root = filepath.Join(home, ".codex")
	} else {
		root = expandTilde(root)
	}
	var doc struct {
		OpenAIAPIKey string `json:"OPENAI_API_KEY"`
		Tokens       struct {
			AccountID   string `json:"account_id"`
			AccessToken string `json:"access_token"`
		} `json:"tokens"`
	}
	if !readJSONFile(filepath.Join(root, "auth.json"), &doc) {
		return false
	}
	return doc.OpenAIAPIKey != "" || doc.Tokens.AccountID != "" || doc.Tokens.AccessToken != ""
}

func fileExists(p string) bool {
	st, err := os.Stat(p)
	return err == nil && !st.IsDir()
}

// readJSONFile decodes a file into v, reporting whether it worked. Never
// surfaces the bytes on failure.
func readJSONFile(p string, v any) bool {
	b, err := os.ReadFile(p)
	if err != nil {
		return false
	}
	return json.Unmarshal(b, v) == nil
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
