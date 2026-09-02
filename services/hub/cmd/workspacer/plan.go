package main

import (
	"fmt"
	"net"
	"net/url"
	"os"
	"strings"
)

// serveOptions is everything `workspacer serve` decides before touching a
// process: resolved binary paths, ports, bind host, token. Kept as a plain
// struct so buildServePlan is a pure function tests can drive table-style.
type serveOptions struct {
	Host              string // hub bind host; claudemon always stays on loopback
	HubPort           int
	APIPort           int // claudemon API / session control
	HookPort          int // claudemon hook ingestion
	Token             string
	ClaudemonBin      string
	HubBin            string
	BrainBin          string // "" = let the hub auto-detect its own sibling
	PluginsDir        string // "" = hub runs without plugins
	WebappDir         string // "" = hub falls back to $WORKSPACER_WEBAPP_DIR
	AdvertiseHost     string // host to print in client URLs (differs from Host when binding 0.0.0.0)
	TrustedHosts      string // comma-separated reverse-proxy hostname(s) for the hub's --trusted-host
	PluginOrigin      string // second origin routed to this hub, for framing plugin UI cross-origin
	DevStreamLogs     bool   // pass --plugins-stream-logs to the hub (plugin dev only)
	SkipClaudemonInit bool   // skip the `claudemon init` pre-flight (operator owns ~/.claude/settings.json)
	DBPath            string // claudemon's SQLite session store; resolved by resolveDBPath, never empty by the time buildServePlan sees it

	// UsagePollOnBoot is the operator's `usage.pollOnBoot` config choice, and
	// nil when the config states none. A pointer rather than a bool because
	// the zero value of this setting is ON: a plain bool would make every
	// serveOptions literal that forgets the field spawn a daemon with account
	// polling switched off. nil leaves the variable unset, which claudemon
	// reads as on.
	UsagePollOnBoot *bool
}

// servePlan is the fully-wired launch plan: the child specs to supervise and
// the endpoints/token to report once healthy.
type servePlan struct {
	// Init is the one-shot pre-flight, run to completion BEFORE the daemons:
	// `claudemon init` merges claudemon's hook + statusLine forwarders into
	// ~/.claude/settings.json. A zero childSpec means the step is off.
	Init      childSpec
	Claudemon childSpec
	Hub       childSpec

	// Health endpoints for the ready wait. claudemon is always on loopback (it
	// is never bound wider), but the HUB is not: --host takes a concrete
	// address, and a concretely-bound listener does NOT answer on 127.0.0.1.
	// A hardcoded loopback probe here tore the whole stack down 20s after a
	// perfectly healthy start and blamed the hub — see dialHost.
	ClaudemonHealth string
	HubHealth       string

	Banner bannerInfo
}

// bannerInfo is what `serve` prints on ready (and what --json emits).
type bannerInfo struct {
	BusURL       string `json:"busUrl"`
	RemoteURL    string `json:"remoteUrl"`
	MobileURL    string `json:"mobileUrl"`
	HubURL       string `json:"hubUrl"`
	ClaudemonURL string `json:"claudemonUrl"`
	Token        string `json:"token"`
}

// buildServePlan wires the port/env/argv plan between the two children:
//
//   - claudemon serves hooks + API on loopback (remote clients never talk to
//     it directly — they go through the hub bus, exactly like the desktop);
//   - the hub binds opts.Host, bridges claudemon's /events onto the bus,
//     points its supervised full-scope brain at the claudemon API, and
//     requires the shared token on /bus + /remote.
//
// The hub passes the token on to the brain itself (HUB_TOKEN), so one --token
// here authenticates the whole tree.
func buildServePlan(opts serveOptions) servePlan {
	apiURL := fmt.Sprintf("http://127.0.0.1:%d", opts.APIPort)

	// THE HOOK-REGISTRATION PRE-FLIGHT. `claudemon init` is a peer subcommand of
	// `serve` (services/claudemon/src/cli.rs), and until this existed the only
	// caller in the whole project was the desktop's Electron main
	// (apps/desktop/src/main/index.ts). `workspacer serve` inherited working
	// hooks on any machine where the desktop had run — they share one
	// ~/.claude/settings.json — and registered nothing at all on a state
	// directory where it had not: a container, a fresh volume, a CI box.
	//
	// What a hookless PTY session does is worse than "no telemetry". A session
	// is born SessionMode::Unknown and ONLY hooks move it (session/state.rs),
	// while a spawn's `first_message` is held until the `Input` transition
	// (session/store.rs, queue_first_message → schedule_pending_flush). So a
	// dispatched worker never receives its prompt: it sits at an empty composer,
	// alive and idle-looking, forever. Permission prompts produce no approvable
	// record either (the note on HookEventKind::PermissionRequest).
	//
	// Note what does NOT break: fleet.quiescence reads `mode: "unknown"` as a
	// blocker, not as rest (internal/quiescence stateBlocker), so the machine
	// stays awake rather than powering down under live work. Failing safe on
	// that axis is why this stayed invisible.
	//
	// Run on every boot, not just the first: init is idempotent — it prints
	// "already up to date" and writes nothing when the merge is a no-op — and
	// the desktop already runs it on every launch, so the cost of matching it is
	// one process that exits immediately.
	initStep := childSpec{
		Name: "claudemon init",
		Bin:  opts.ClaudemonBin,
		Args: []string{"init", "--hook-port", fmt.Sprintf("%d", opts.HookPort)},
	}
	if opts.SkipClaudemonInit {
		initStep = childSpec{}
	}

	claudemon := childSpec{
		Name: "claudemon",
		Bin:  opts.ClaudemonBin,
		Args: []string{
			"serve",
			"--host", "127.0.0.1",
			"--hook-port", fmt.Sprintf("%d", opts.HookPort),
			"--api-port", fmt.Sprintf("%d", opts.APIPort),
		},
	}
	// PIN THE DATABASE LIKE A PORT. Every port claudemon binds was named here
	// and the one piece of persistent state it opens was not, so the daemon
	// resolved it privately (store/mod.rs default_db_path) and nothing in the
	// plan — or in the banner, or in a test — could say which file a stack was
	// about to write. That is how two stacks on alternate ports came to share
	// one `sessions`/`events` table in silence. resolveDBPath has already
	// decided (and refused the sharing case); this only carries the answer.
	if opts.DBPath != "" {
		claudemon.Args = append(claudemon.Args, "--db-path", opts.DBPath)
	}
	// Match the desktop's default daemon verbosity, but let the user's own
	// RUST_LOG win (childSpec env is appended after os.Environ, overriding it,
	// so only set ours when the user didn't).
	if os.Getenv("RUST_LOG") == "" {
		claudemon.Env = []string{"RUST_LOG=claudemon=info"}
	}
	// usage.pollOnBoot. claudemon's account-usage poller decides at boot
	// whether to iterate every configured Claude root or only the roots of live
	// sessions, and the daemon has no config file — the setting travels in its
	// environment, the same variable and the same 0/1 spelling the desktop
	// writes (apps/desktop/src/main/services/claudemonDaemon.ts usagePollEnv).
	// Unset when the config states nothing, which the daemon reads as ON.
	if opts.UsagePollOnBoot != nil {
		v := "1"
		if !*opts.UsagePollOnBoot {
			v = "0"
		}
		claudemon.Env = append(claudemon.Env, "WORKSPACER_USAGE_POLL_ON_BOOT="+v)
	}

	hubArgs := []string{
		"--addr", net.JoinHostPort(opts.Host, fmt.Sprintf("%d", opts.HubPort)),
		"--claudemon-events", apiURL + "/events",
		"--claudemon", apiURL,
		"--brain-scope", "full",
		"--token", opts.Token,
	}
	if opts.BrainBin != "" {
		hubArgs = append(hubArgs, "--brain-bin", opts.BrainBin)
	}
	if opts.PluginsDir != "" {
		hubArgs = append(hubArgs, "--plugins-dir", opts.PluginsDir)
	}
	if opts.WebappDir != "" {
		hubArgs = append(hubArgs, "--webapp-dir", opts.WebappDir)
	}
	if opts.DevStreamLogs {
		hubArgs = append(hubArgs, "--plugins-stream-logs")
	}
	if opts.PluginOrigin != "" {
		// A browser cannot be served /app and a plugin's UI on ONE origin without
		// handing that plugin the app document, so /app frames a same-origin
		// plugin opaque and the plugin loses its bus link. A second origin the
		// operator already routes here dissolves that without loosening anything.
		hubArgs = append(hubArgs, "--plugin-origin", opts.PluginOrigin)
	}
	if opts.TrustedHosts != "" {
		// A TLS front-end (tailscale serve, nginx, Caddy) terminates elsewhere
		// and forwards to the hub, so it presents a Host the hub's rebinding
		// pin refuses. Naming it is the operator's opt-in.
		hubArgs = append(hubArgs, "--trusted-host", opts.TrustedHosts)
	}
	hub := childSpec{Name: "hub", Bin: opts.HubBin, Args: hubArgs}

	adv := opts.AdvertiseHost
	if adv == "" {
		adv = opts.Host
	}
	hubHostPort := net.JoinHostPort(adv, fmt.Sprintf("%d", opts.HubPort))
	q := "?token=" + url.QueryEscape(opts.Token)

	return servePlan{
		Init:            initStep,
		Claudemon:       claudemon,
		Hub:             hub,
		ClaudemonHealth: apiURL + "/health",
		HubHealth:       fmt.Sprintf("http://%s/health", net.JoinHostPort(dialHost(opts.Host), fmt.Sprintf("%d", opts.HubPort))),
		Banner: bannerInfo{
			BusURL:       "ws://" + hubHostPort + "/bus",
			RemoteURL:    "http://" + hubHostPort + "/remote" + q,
			MobileURL:    "http://" + hubHostPort + "/m" + q,
			HubURL:       "http://" + hubHostPort,
			ClaudemonURL: apiURL,
			Token:        opts.Token,
		},
	}
}

// dialHost is the host THIS process must dial to reach a child bound to
// bindHost. A wildcard bind names no host, so dialing it is meaningless (that
// is the bug that cost the brain its whole capability plane — see
// cmd/hub/brain.go busDialAddr); loopback is the right probe for it. Any
// CONCRETE host must be dialed as itself: `--host 100.86.79.73` (the tailnet
// form this flag's own help recommends) does not answer on 127.0.0.1, so a
// hardcoded loopback probe never reaches a hub that came up healthy.
func dialHost(bindHost string) string {
	switch bindHost {
	case "", "0.0.0.0", "::", "[::]":
		return "127.0.0.1"
	}
	return bindHost
}

// advertiseHost picks the host to print in client URLs. A concrete bind host
// is advertised as-is; a wildcard bind (0.0.0.0 / ::) is useless in a URL, so
// we pick from the machine's IPv4 addresses, preferring a Tailscale CGNAT
// address (100.64.0.0/10) — remote sharing is Tailscale-intended — then any
// non-loopback IPv4, then loopback. Pure over the candidate list so tests
// don't depend on the host's real interfaces.
func advertiseHost(bindHost string, ipv4s []string) string {
	if bindHost != "" && bindHost != "0.0.0.0" && bindHost != "::" {
		return bindHost
	}
	fallback := ""
	for _, ip := range ipv4s {
		if isTailscaleIPv4(ip) {
			return ip
		}
		if fallback == "" {
			fallback = ip
		}
	}
	if fallback != "" {
		return fallback
	}
	return "127.0.0.1"
}

// isTailscaleIPv4 reports whether ip falls in the CGNAT range Tailscale
// assigns (100.64.0.0/10). A plain "100." prefix check (what the desktop does)
// would also match public 100.0.x.x space, so mask properly here.
func isTailscaleIPv4(ip string) bool {
	p := net.ParseIP(ip)
	if p == nil {
		return false
	}
	_, cgnat, _ := net.ParseCIDR("100.64.0.0/10")
	return cgnat.Contains(p)
}

// localIPv4s lists the machine's non-loopback IPv4 addresses, for
// advertiseHost when binding a wildcard.
func localIPv4s() []string {
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		return nil
	}
	var out []string
	for _, a := range addrs {
		ipnet, ok := a.(*net.IPNet)
		if !ok || ipnet.IP.IsLoopback() {
			continue
		}
		if v4 := ipnet.IP.To4(); v4 != nil {
			out = append(out, v4.String())
		}
	}
	return out
}

// renderBanner renders the human-readable ready banner. The token is printed
// deliberately: unlike the desktop (which hides it in the Remote control
// panel), a headless server's terminal IS the pairing surface.
func renderBanner(b bannerInfo) string {
	var sb strings.Builder
	sb.WriteString("\nworkspacer server ready\n\n")
	rows := [][2]string{
		{"bus", b.BusURL},
		{"remote", b.RemoteURL},
		{"mobile", b.MobileURL},
		{"claudemon", b.ClaudemonURL},
		{"token", b.Token + "  (operator pairing credential — anyone who has it controls this server)"},
	}
	for _, r := range rows {
		fmt.Fprintf(&sb, "  %-10s %s\n", r[0], r[1])
	}
	sb.WriteString("\nScoped tokens (read-only / triage instead of full control): workspacer token create --scope view|triage\n")
	sb.WriteString("Press Ctrl-C to stop.\n")
	return sb.String()
}
