// Package authtoken is the scoped capability-token store: bearer tokens that
// authenticate like the host remote-token but are *authorized* for only a tier
// of bus methods. Tokens live in <config>/workspacer/tokens.json — right next
// to the host `remote-token` — so the CLI (`workspacer token …`), the desktop
// app, and the hub all agree on one file. The hub loads it and enforces the
// scope at the router's single dispatch point; the legacy remote-token keeps
// full access (it has no scope record — implicit operator), so existing
// pairings never break.
package authtoken

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"
)

// Scope is a grant tier. Tiers are expressed as method patterns (exact names or
// `prefix.*` globs, matched with internal/event.Matches), so a method that
// doesn't appear in a tier — including any method added later — fails closed
// for tokens of that tier.
type Scope string

const (
	// ScopeView is read-only: lists, snapshots, transcripts, event/stream
	// subscriptions, and getCwd-style introspection. Nothing that changes state.
	ScopeView Scope = "view"
	// ScopeTriage is view plus acting on attention: approve/deny, answer,
	// send a message, interrupt, and the Web Push subscription methods the /m
	// PWA needs. NOT spawn, NOT terminals, NOT git mutations, NOT plugin or
	// config admin.
	ScopeTriage Scope = "triage"
	// ScopeOperator is everything — equivalent to the host remote-token.
	ScopeOperator Scope = "operator"
)

// ParseScope validates a user-supplied scope name.
func ParseScope(s string) (Scope, error) {
	switch Scope(strings.ToLower(strings.TrimSpace(s))) {
	case ScopeView:
		return ScopeView, nil
	case ScopeTriage:
		return ScopeTriage, nil
	case ScopeOperator:
		return ScopeOperator, nil
	}
	return "", fmt.Errorf("unknown scope %q (want view, triage, or operator)", s)
}

// viewMethods is the read-only surface, derived from what the read paths of
// the real clients actually call (cmd/hub/mobile.html, cmd/hub/remote.html,
// apps/desktop webBackend.ts, cmd/mcp). Exact names on purpose: a broad
// `agents.*` would silently grant agents.spawn, and any method added later
// must be admitted here deliberately (fail closed for scoped tokens).
var viewMethods = []string{
	"agents.list",           // fleet list (/remote, MCP list_agents)
	"sessions.snapshots",    // full fleet snapshot seed (/m, webBackend)
	"sessions.snapshot",     // one session's snapshot (webBackend, MCP)
	"sessions.transcript",   // transcript reads (/remote, MCP get_transcript)
	"sessions.conversation", // normalized conversation reads (MCP, webBackend)
	"layout.get",            // shared workspace layout document (read side)
	"config.get",            // /m reads UI config at boot (read-only twin of config.save)
	"app.getCwd",            // getCwd-style introspection (MCP, webBackend)
	"push.key",              // VAPID public key — needed before subscribing, discloses nothing
}

// triageMethods is what "acting on attention" adds on top of view: resolving
// the asks an agent is blocked on, talking to it, interrupting it, and the Web
// Push subscription the /m PWA uses to hear about those asks in the
// background. Deliberately absent: agents.spawn (the /m spawn tab is operator
// surface), terminals.*, git.*, fs.*, config.save, plugin/config admin.
var triageMethods = []string{
	"claude.approve",     // permission prompts — yes / no / always (/m, /remote)
	"claude.answer",      // AskUserQuestion pickers (/m, /remote)
	"agents.sendMessage", // send a prompt / reply to an agent (/m chat)
	"claude.signal",      // interrupt a runaway agent (/m + /remote SIGINT button)
	"push.subscribe",     // /m PWA background notifications
	"push.unsubscribe",   // symmetric teardown of the same subscription
}

// Methods returns the method patterns a scope may call. Operator is the single
// wildcard; the scoped tiers are explicit allowlists that fail closed for
// anything unlisted.
func (s Scope) Methods() []string {
	switch s {
	case ScopeView:
		return append([]string(nil), viewMethods...)
	case ScopeTriage:
		out := append([]string(nil), viewMethods...)
		return append(out, triageMethods...)
	case ScopeOperator:
		return []string{"*"}
	}
	return nil // unknown scope grants nothing — fail closed
}

// Record is one persisted scoped token.
type Record struct {
	Token   string    `json:"token"`
	Scope   Scope     `json:"scope"`
	Label   string    `json:"label,omitempty"`
	Created time.Time `json:"created"`
}

// ConfigDir mirrors the desktop app's getConfigDir (configService.ts) and the
// CLI's configDir: %APPDATA%\workspacer on Windows, $XDG_CONFIG_HOME/workspacer
// or ~/.config/workspacer elsewhere. Sharing the directory is deliberate — the
// token file must sit next to the remote-token every component already reads.
func ConfigDir() string {
	if runtime.GOOS == "windows" {
		if appData := os.Getenv("APPDATA"); appData != "" {
			return filepath.Join(appData, "workspacer")
		}
		home, _ := os.UserHomeDir()
		return filepath.Join(home, "AppData", "Roaming", "workspacer")
	}
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "workspacer")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".config", "workspacer")
}

// DefaultPath is where scoped tokens persist: <config>/workspacer/tokens.json,
// next to remote-token.
func DefaultPath() string {
	return filepath.Join(ConfigDir(), "tokens.json")
}

// Load reads the token file. A missing file is an empty store, not an error.
func Load(path string) ([]Record, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var recs []Record
	if err := json.Unmarshal(b, &recs); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return recs, nil
}

// Save writes the token file with owner-only permissions (it holds bearer
// secrets, like remote-token), via a temp file + rename so a crash mid-write
// can't truncate existing tokens.
func Save(path string, recs []Record) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	b, err := json.MarshalIndent(recs, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".tokens-*.json")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op after a successful rename
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(append(b, '\n')); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmpName, path)
}

// Mint creates, persists, and returns a new scoped token. The token has the
// same shape as the host remote-token (24 random bytes, base64url).
func Mint(path string, scope Scope, label string) (Record, error) {
	if _, err := ParseScope(string(scope)); err != nil {
		return Record{}, err
	}
	recs, err := Load(path)
	if err != nil {
		return Record{}, err
	}
	raw := make([]byte, 24)
	if _, err := rand.Read(raw); err != nil {
		return Record{}, err
	}
	rec := Record{
		Token:   base64.RawURLEncoding.EncodeToString(raw),
		Scope:   scope,
		Label:   label,
		Created: time.Now().UTC().Truncate(time.Second),
	}
	if err := Save(path, append(recs, rec)); err != nil {
		return Record{}, err
	}
	return rec, nil
}

// Revoke removes a token by exact value or by unique prefix (min 8 chars, so a
// `workspacer token list` snippet is enough). Returns the removed record.
// Ambiguous or unknown references are errors, never silent no-ops.
func Revoke(path, ref string) (Record, error) {
	ref = strings.TrimSpace(ref)
	if len(ref) < 8 {
		return Record{}, fmt.Errorf("token reference %q too short (give the full token or ≥8 leading characters)", ref)
	}
	recs, err := Load(path)
	if err != nil {
		return Record{}, err
	}
	idx := -1
	for i, r := range recs {
		if r.Token == ref || strings.HasPrefix(r.Token, ref) {
			if idx != -1 {
				return Record{}, fmt.Errorf("prefix %q matches more than one token", ref)
			}
			idx = i
		}
	}
	if idx == -1 {
		return Record{}, fmt.Errorf("no token matching %q", ref)
	}
	removed := recs[idx]
	recs = append(recs[:idx], recs[idx+1:]...)
	if err := Save(path, recs); err != nil {
		return Record{}, err
	}
	return removed, nil
}

// Store is a read-through cache over the token file for the hub's handshake
// path. Lookup re-reads the file when its mtime/size changed, so `workspacer
// token create` / `token revoke` take effect on the next connection without
// restarting the hub or adding a minting endpoint. Revoking also cuts off a
// token's *future* connections; a connection already open keeps its grants
// until it drops (same as rotating the host remote-token today).
type Store struct {
	path string

	mu      sync.Mutex
	mtime   time.Time
	size    int64
	loaded  bool
	byToken map[string]Record
}

// NewStore wraps a token file path. The file need not exist.
func NewStore(path string) *Store {
	return &Store{path: path}
}

// Lookup resolves a presented bearer token to its scope record.
func (st *Store) Lookup(token string) (Record, bool) {
	if token == "" {
		return Record{}, false
	}
	st.mu.Lock()
	defer st.mu.Unlock()
	st.refreshLocked()
	rec, ok := st.byToken[token]
	return rec, ok
}

func (st *Store) refreshLocked() {
	info, err := os.Stat(st.path)
	if err != nil {
		// Missing (or unreadable) file = empty store. Fail closed: no scoped
		// tokens are honored rather than stale ones.
		st.byToken = nil
		st.loaded = true
		st.mtime, st.size = time.Time{}, 0
		return
	}
	if st.loaded && info.ModTime().Equal(st.mtime) && info.Size() == st.size {
		return
	}
	recs, err := Load(st.path)
	if err != nil {
		// Corrupt file: honor nothing from it (fail closed), but leave loaded
		// state so we retry once it changes again.
		recs = nil
	}
	m := make(map[string]Record, len(recs))
	for _, r := range recs {
		if r.Token != "" {
			m[r.Token] = r
		}
	}
	st.byToken = m
	st.loaded = true
	st.mtime, st.size = info.ModTime(), info.Size()
}
