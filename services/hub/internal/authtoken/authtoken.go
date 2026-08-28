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
	"os/user"
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
	// ScopeTriage is view plus acting on attention: approve/deny, send a
	// message, interrupt, and the Web Push subscription methods the /m PWA
	// needs. NOT spawn, NOT terminals, NOT git mutations, NOT plugin or config
	// admin, and NOT claude.answer (a raw-PTY-write twin of terminalInput).
	ScopeTriage Scope = "triage"
	// ScopeOperator is everything — equivalent to the host remote-token.
	ScopeOperator Scope = "operator"
	// ScopeProvider is NOT a rung on the view ⊂ triage ⊂ operator ladder. That
	// ladder is a PERSON's authority, each rung a bigger set of methods a human
	// client may call. This is an orthogonal axis: a headless capability
	// PROVIDER — `brain --hub wss://…` on a remote node — which answers calls
	// rather than making them.
	//
	// It holds one method (layout.get, which view holds too, so its call
	// surface is a strict SUBSET of the smallest human tier) and adds one
	// authority no human tier has: registering as the answerer of a call, per
	// the token record's own Provides grant. It consumes NOTHING — see
	// providerMethods — and it may publish only a topic whose publishing
	// capability it provides (capspec.EventTopic.Publisher, enforced by the
	// bus's mayPublish).
	//
	// It exists because before it, the ONLY credential shape that could
	// register a capability without a plugin manifest was operator — which the
	// bus promotes to `trusted`, i.e. host authority: nodes.wake (spends
	// money), jobs.*, POST /plugins/install, the whole event firehose including
	// other machines' raw PTY bytes, and forging any host-owned topic. A remote
	// node needed one authority and was handed nine.
	ScopeProvider Scope = "provider"
)

// asciiWhitespace is the surrounding-whitespace set ParseScope trims, spelled as
// a literal rather than delegated to strings.TrimSpace — because TrimSpace
// (unicode.IsSpace) and JS String.prototype.trim, which the desktop's
// normalizeScope twin uses, are NOT the same function and disagree on exactly two
// code points that reach a caller-supplied scope: U+FEFF (BOM — trimmed by JS,
// not by Go) and U+0085 (NEL — trimmed by Go, not by JS). Left to the built-ins,
// a BOM-wrapped "operator" parsed to operator on the desktop and was refused
// here, while a NEL-wrapped one did the opposite: the SAME scope string minting
// grants depending on which stack answered. Trimming the ASCII set on both sides
// makes every non-ASCII wrapper fail closed identically.
// TWIN: apps/desktop/src/main/lib/asciiWhitespace.ts (trimAsciiWhitespace, used
// by remoteTokens.ts normalizeScope); same set as cmd/brain/profiles.go.
const asciiWhitespace = " \t\n\v\f\r"

// ParseScope validates a user-supplied scope name.
func ParseScope(s string) (Scope, error) {
	switch Scope(strings.ToLower(strings.Trim(s, asciiWhitespace))) {
	case ScopeView:
		return ScopeView, nil
	case ScopeTriage:
		return ScopeTriage, nil
	case ScopeOperator:
		return ScopeOperator, nil
	case ScopeProvider:
		return ScopeProvider, nil
	}
	return "", fmt.Errorf("unknown scope %q (want view, triage, operator, or provider)", s)
}

// viewMethods is the read-only surface, derived from what the read paths of
// the real clients actually call (cmd/hub/mobile.html, cmd/hub/remote.html,
// apps/desktop webBackend.ts, cmd/mcp). Exact names on purpose: a broad
// `agents.*` would silently grant agents.spawn, and any method added later
// must be admitted here deliberately (fail closed for scoped tokens).
var viewMethods = []string{
	"agents.list",                   // fleet list (/remote, MCP list_agents)
	"sessions.snapshots",            // full fleet snapshot seed (/m, webBackend)
	"sessions.snapshot",             // one session's snapshot (webBackend, MCP)
	"sessions.recent",               // resumable-session list for the Sessions pane (webBackend)
	"sessions.transcript",           // transcript reads (/remote, MCP get_transcript)
	"sessions.conversation",         // normalized conversation reads (MCP, webBackend)
	"sessions.subagentConversation", // provider-owned child-thread conversation reads (webBackend)
	"layout.get",                    // shared workspace layout document (read side)
	"config.get",                    // /m reads UI config at boot (read-only twin of config.save)
	"app.getCwd",                    // getCwd-style introspection (MCP, webBackend)
	"federation.peers",              // peer-hub names + connected bit — the /m PWA and web
	//                          renderer seed the federated fleet from it, and the
	//                          names are already stamped on every agent.* event
	//                          this same tier receives
	"fleet.quiescence", // is this machine's fleet at rest, and if not, what is
	//                     holding it up. A read the phone and the web renderer
	//                     could already assemble one poll at a time from the
	//                     snapshot feed this tier receives; nothing in the answer
	//                     names a credential, an address, or a job's argv
	"nodes.list", // the remote-node registry: which machines exist and whether
	//                each is available, waking, stopped or unreachable. Admitted
	//                for the same reason federation.peers is — it is a TOMBSTONE
	//                signal, and a client that cannot read it renders a node that
	//                is asleep on purpose as one that has died. What it hands back
	//                is nodes.NodeView, an allowlist projection: a label, a state,
	//                two timestamps and a sentence. It names no credential, no
	//                cloud app, no machine id and no endpoint.
	//
	//                Its two ACTING twins, nodes.wake and nodes.sleep, are
	//                deliberately NOT here or in triage. Waking starts a billable
	//                machine; sleeping ends the work in flight on one somebody may
	//                be typing at. Both are host-authority only (nodesTrusted in
	//                cmd/hub), and note that the second is refused for a reason of
	//                its OWN rather than by symmetry — "it only turns things off"
	//                is a destructive act, not a smaller one. A phone on this tier
	//                SEES the state and gets neither button, which /m already
	//                renders gracefully via can().
	"push.key", // VAPID public key — needed before subscribing, discloses nothing
	// The one method in this list that WRITES anywhere, admitted so the facade's
	// tool set derives from this allowlist again rather than sitting beside it.
	// It is here because it is the least a tier can hold and still let a
	// read-only worker say "the approach you gave me is wrong": before it, that
	// sentence cost a triage dispatch, which also hands the worker approve and
	// interrupt over OTHER sessions.
	//
	// What makes it admissible at VIEW is that a caller on this tier cannot
	// reach it. It names no recipient — the host derives that from the caller's
	// own parentSessionId — and it cannot name a SENDER either: `callerSessionId`
	// is the caller asserting who it is, and the router deletes that from every
	// untrusted caller (sanitizeReportProgressParams, on the local AND the
	// federated dispatch path). A scoped bus connection carries no session
	// identity to stamp from, so a phone or plugin token calling this directly
	// lands on the provider's "could not identify your session" refusal. Only
	// the MCP facade — which resolves the session from the per-request token
	// record's `session:<id>` label — can actually use it. Acknowledged as a
	// deliberate view/triage actor in authtoken's composition_test.go.
	"agents.reportProgress",
}

// triageMethods is what "acting on attention" adds on top of view: resolving
// the asks an agent is blocked on, talking to it, interrupting it, and the Web
// Push subscription the /m PWA uses to hear about those asks in the
// background. Deliberately absent: agents.spawn (the /m spawn tab is operator
// surface), terminals.*, git.*, fs.*, config.save, layout.set, plugin/config
// admin — and claude.answer, whose PTY path types `text + "\r"` into the
// session's PTY through the SAME r.cm.input sink sessions.terminalInput uses,
// with no pending-question requirement and no ownership check on the sessionId
// (capspec classifies its text/answers/option as KindShell for exactly this
// reason). A session id can name a terminals.create shell as readily as an
// agent, so granting it would hand a phone token raw host keystrokes onto
// /bin/bash — the very primitive sessions.terminalInput is excluded to withhold.
// The /m AskUserQuestion picker therefore stays operator surface; a phone
// answers by talking to the agent (sendMessage) or resolving its permission
// prompt (approve), never by a raw PTY write.
//
// AND THE PARAGRAPH ABOVE IS NOT THE WHOLE TRUTH ON ITS OWN. Every one of those
// absences is real and enforced per call — a triage token cannot run a shell,
// write a file, commit, spawn, or change settings. But two of the methods it
// DOES hold compose past all of them: `agents.sendMessage` injects an arbitrary
// prompt into an agent already running as the desktop user, and `claude.approve`
// resolves the permission prompt that agent then raises (with
// decision:"always" persisting a standing allow, so later calls of that tool are
// not parked at all). capspec excuses agents.sendMessage on the grounds that
// "the agent's own tool approvals are the gate", and this tier holds the
// resolver of exactly those approvals; claude.approve's own entry already
// concedes it is "the RESOLVER of the approvals agents.sendMessage's own excuse
// rests on".
//
// That is a deliberate product decision — the phone's whole job is replying to
// an agent and answering what it asks — so it is ON THE RECORD as
// capspec.Compositions()'s only AcceptedIn entry rather than pretended away, and
// composition_test.go fails if any tier acquires both halves of a pair it was
// not accepted for. `claude.gate` is deliberately NOT here: gate only ADDS
// parking, so the pair never needed it, and its absence is pinned so that
// "the phone should be able to arm the gate" lands in a test rather than
// quietly changing what a phone token is.
var triageMethods = []string{
	"claude.approve",     // permission prompts — yes / no / always (/m, /remote)
	"agents.sendMessage", // send a prompt / reply to an agent (/m chat)
	"claude.signal",      // interrupt a runaway agent (/m + /remote SIGINT button)
	"push.subscribe",     // /m PWA background notifications
	"push.unsubscribe",   // symmetric teardown of the same subscription
	"push.test",          // "is push reaching me?" — the tier that subscribes must be able to check
	"files.upload",       // land a photo from the phone on this machine's tmp so a message can reference it
}

// providerMethods is the ENTIRE outbound call surface of a headless capability
// provider, derived the same way viewMethods was — from what the real client
// actually calls. `brain --scope full` registers 67 methods and answers them;
// grepping every call site in cmd/brain for what it CALLS finds exactly one:
// layout.get (cmd/brain/main.go), the fleet-visibility rule's backing read.
//
// One method, and it is already in viewMethods, so a provider token's call
// surface is a strict subset of the smallest human tier's. Everything a node
// does that looks like authority — registering 67 capabilities, streaming PTY
// bytes, being woken — is either the register grant (Record.Provides) or
// something the node does not do at all: the hub starts the machine through the
// Fly Machines API and then PROBES the node with brain.info over its own
// loopback client, so "receiving wakes" and "reporting state" are not bus acts
// the node performs.
//
// Nothing else goes in here without a call site in cmd/brain to justify it. A
// provider that needs to CALL something is asking for the operator ladder, and
// that is a different question from being allowed to ANSWER.
var providerMethods = []string{
	"layout.get", // cmd/brain/main.go — the only method a headless node calls
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
	case ScopeProvider:
		return append([]string(nil), providerMethods...)
	}
	return nil // unknown scope grants nothing — fail closed
}

// Record is one persisted scoped token.
type Record struct {
	Token   string    `json:"token"`
	Scope   Scope     `json:"scope"`
	Label   string    `json:"label,omitempty"`
	Created time.Time `json:"created"`
	// Plugins lists the plugin ids whose contributed MCP-facade tools this
	// token may use (opt-in per token; empty = none). Read only by the facade —
	// the hub bus ignores it, because plugin-provided methods are gated by the
	// PROVIDING plugin's own grants, and a scoped token's Methods() never
	// includes a plugin namespace.
	Plugins []string `json:"plugins,omitempty"`
	// ProfilesAllowed lists the Claude profile ids this token may dispatch
	// agents under: an agents.spawn naming a profileId in this list keeps it
	// (the hub stamps `profileGranted` beside it); any other profileId is
	// stripped before the call reaches a provider. Same philosophy as Plugins —
	// a grant recorded at mint time by the host user (the desktop's
	// fleet-manager spawn path), never claimable by the caller itself. Exact
	// ids only, deliberately no "*": blessing a manager means naming the
	// accounts it may burn. Enforced in BOTH places tokens are verifiable: the
	// hub router (scoped bus connections) and the MCP facade (per-session
	// records multiplexed over the facade's own trusted connection).
	ProfilesAllowed []string `json:"profilesAllowed,omitempty"`
	// YoloAllowed is the full-access grant: an agents.spawn from this token may
	// have its `skipPermissions` request HONORED (--dangerously-skip-permissions
	// / a bypass permissionMode) instead of clamped. Like ProfilesAllowed, it is
	// recorded at mint time by the host user (the desktop's fleet-manager spawn
	// path, gated by the agents.fleetFullAccess setting), never claimable by the
	// caller itself. The hub router is the sole stamper: sanitizeSpawnParams
	// deletes any incoming `yoloGranted` and re-adds it only for a verified
	// grant (or the trusted host), so a provider seeing the stamp knows the hub
	// judged the caller — the stamp says the request MAY be honored, it does not
	// itself request the bypass.
	YoloAllowed bool `json:"yoloAllowed,omitempty"`
	// Role tags a session token with the role of the session it was minted for
	// ("manager"). Written by the desktop's mint path and read back by its
	// grant reconciler (a config full-access flip updates exactly the manager
	// session tokens, live, in both directions). The Go
	// side never acts on it — the field exists here so the CLI's Load→Save
	// rewrites (token create/revoke) preserve it instead of silently stripping
	// every session token's role. TWIN: RemoteTokenRecord.role (ipcTypes.ts).
	Role string `json:"role,omitempty"`
	// Provides is the REGISTER grant: the capability-method patterns a
	// ScopeProvider connection may register as the provider of. It is the
	// second source for the bus's cn.provides — until it existed, the plugin
	// manifest was the ONLY source, which is precisely why registering a
	// capability without a manifest required a token the bus promotes to
	// trusted. Read only for a provider record (see ProvidesGrant): a `provides`
	// hand-written onto a view or triage record grants nothing.
	//
	// WHY THE MINT DEFAULT IS ["*"] AND NARROWING IS NOT EXPOSED. If this grant
	// is narrower than what the provider tries to register, the hub silently
	// withholds the rest and the brain re-sends `register` every 5 seconds
	// forever (cmd/brain/bus.go registerRetryInterval): the `registered` ack
	// carries the accepted list and no reason, so the retry loop cannot tell
	// "another live connection owns this method" (retrying is correct — the
	// owner may drop) from "your token may not have this" (retrying is a
	// permanent busy loop). Until the ack can say WHICH, a narrowed grant is a
	// footgun that presents as a working node with a hot 5s loop, so Mint
	// writes ["*"] and `workspacer token create` exposes no --provides flag.
	// The field is in the wire format now so the record shape is settled and
	// the desktop's twin preserves it — see remoteTokens.ts normalizeRecord.
	//
	// ["*"] here is NOT the operator "*". That one lives in Methods and is read
	// as host identity by ScopedIdent.operator(); this one is read only by
	// mayProvide. The two lists must never be merged — see the bus package's
	// TestProviderTokenProvidesStarIsNotOperatorStar.
	// TWIN: RemoteTokenRecord.provides (ipcTypes.ts).
	Provides []string `json:"provides,omitempty"`
}

// ProvidesGrant is Record.Provides as the BUS should read it: the register
// grant, or nil for any tier that has no business registering capabilities.
//
// The tier is the gate, not the field. tokens.json is a plain file the CLI, the
// desktop and the hub all rewrite, so `provides` appearing on a view record —
// a hand edit, a bad migration, a merge of two stores — must grant nothing
// rather than quietly make a read-only phone token the answerer of
// claude.approve. Callers pass THIS to bus.ScopedIdent, never the raw field.
func (r Record) ProvidesGrant() []string {
	if r.Scope != ScopeProvider {
		return nil
	}
	return append([]string(nil), r.Provides...)
}

// ConfigDir mirrors the desktop app's getConfigDir (configService.ts) and the
// CLI's configDir: %APPDATA%\workspacer on Windows, $XDG_CONFIG_HOME/workspacer
// or ~/.config/workspacer elsewhere. Sharing the directory is deliberate — the
// token file must sit next to the remote-token every component already reads.
// Returns "" when there is no home directory to anchor on — see HomeDir. That
// is the refusal: internal/bus's secret gate canonicalizes this root and
// discards anything non-absolute, so "" fails closed where a RELATIVE
// ".config/workspacer" silently named a directory under the daemon's cwd.
func ConfigDir() string {
	if runtime.GOOS == "windows" {
		if appData := os.Getenv("APPDATA"); appData != "" {
			return filepath.Join(appData, "workspacer")
		}
		home := HomeDir()
		if home == "" {
			return ""
		}
		return filepath.Join(home, "AppData", "Roaming", "workspacer")
	}
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "workspacer")
	}
	home := HomeDir()
	if home == "" {
		return ""
	}
	return filepath.Join(home, ".config", "workspacer")
}

// HomeDir is os.UserHomeDir with the fallback Node's os.homedir() has and Go's
// does not: the effective uid's passwd entry.
//
// os.UserHomeDir reads $HOME and nothing else, and every caller here discarded
// its error — so under a systemd unit, a launchd job or a container entrypoint
// with no HOME, ConfigDir answered the RELATIVE ".config/workspacer" while the
// desktop and the TUI (Node / Rust, both of which consult passwd) went on using
// the real one. Same rule, three copies, one of them off by a whole directory.
//
// Exported because cmd/brain's configDirFor carries the twin of this comment and
// the two are pinned against each other by a test.
func HomeDir() string {
	if h, err := os.UserHomeDir(); err == nil && h != "" {
		return h
	}
	if u, err := user.Current(); err == nil && u.HomeDir != "" {
		return u.HomeDir
	}
	return ""
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

// Mint creates, persists, and returns a new scoped token with no extra grants.
// The token has the same shape as the host remote-token (24 random bytes,
// base64url).
func Mint(path string, scope Scope, label string) (Record, error) {
	return MintGranted(path, scope, label, false)
}

// MintGranted is [Mint] with the full-access grant decided at mint time.
//
// `yoloAllowed` is the ONLY way to hand a credential the right to spawn agents
// that skip approvals, and it exists as a mint-time argument because the two
// callers that need it cannot use the desktop's fleet-manager mint path:
//
//   - a FEDERATION LINK's token. A peer link inherits no host trust (see
//     bus.conn.mayBypassPermissions), so a hub that wants its peer to dispatch
//     full-access work must mint the link a token that SAYS so and put that
//     token in the peer's peers.json entry.
//   - a headless node with no desktop attached, where there is no
//     agents.fleetFullAccess UI to flip.
//
// It is deliberately not settable by the holder, and never inferred: a token
// carries the grant because a human at this machine typed the flag.
func MintGranted(path string, scope Scope, label string, yoloAllowed bool) (Record, error) {
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
		Token:       base64.RawURLEncoding.EncodeToString(raw),
		Scope:       scope,
		Label:       label,
		Created:     time.Now().UTC().Truncate(time.Second),
		YoloAllowed: yoloAllowed,
	}
	if scope == ScopeProvider {
		// The default is the WHOLE register surface, deliberately, and it is not
		// laziness: a grant narrower than what the provider registers puts the
		// brain in a permanent 5s re-register loop, because the `registered` ack
		// says WHAT was accepted and never WHY the rest was withheld — so the
		// retry cannot distinguish "owned by another live connection, retrying
		// is right" from "refused by your grant, retrying is forever". See
		// Record.Provides.
		//
		// Wide here is still nine authorities narrower than the operator token
		// this replaces: it is "may ANSWER any call", not "may MAKE any call",
		// and it carries no publish-anything, no consume-anything, no
		// Server.Authorized, no nodes.wake and no jobs.*.
		rec.Provides = []string{"*"}
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

// HasFingerprint reports whether a live token in the store fingerprints to id
// under fp. It exists for state that OUTLIVES the connection that created it —
// a Web Push subscription is registered once and then notified forever, so
// revocation has to be re-checked against the credential each time rather than
// at handshake. The store holds raw tokens and never the fingerprints, and the
// fingerprint function belongs to the bus, so the caller passes it in: that
// keeps this package free of a bus import while guaranteeing both sides compute
// the same string. Re-reads the file first, so `workspacer token revoke` takes
// effect on the next notification with no restart.
func (st *Store) HasFingerprint(id string, fp func(string) string) bool {
	if id == "" || fp == nil {
		return false
	}
	st.mu.Lock()
	defer st.mu.Unlock()
	st.refreshLocked()
	for tok := range st.byToken {
		if fp(tok) == id {
			return true
		}
	}
	return false
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
