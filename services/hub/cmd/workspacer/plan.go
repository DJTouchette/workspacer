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
	Host          string // hub bind host; claudemon always stays on loopback
	HubPort       int
	APIPort       int // claudemon API / session control
	HookPort      int // claudemon hook ingestion
	Token         string
	ClaudemonBin  string
	HubBin        string
	BrainBin      string // "" = let the hub auto-detect its own sibling
	PluginsDir    string // "" = hub runs without plugins
	WebappDir     string // "" = hub falls back to $WORKSPACER_WEBAPP_DIR
	AdvertiseHost string // host to print in client URLs (differs from Host when binding 0.0.0.0)
}

// servePlan is the fully-wired launch plan: the child specs to supervise and
// the endpoints/token to report once healthy.
type servePlan struct {
	Claudemon childSpec
	Hub       childSpec

	// Loopback health endpoints for the ready wait — always 127.0.0.1 even
	// when the hub binds wider, because we're probing our own children.
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
	// Match the desktop's default daemon verbosity, but let the user's own
	// RUST_LOG win (childSpec env is appended after os.Environ, overriding it,
	// so only set ours when the user didn't).
	if os.Getenv("RUST_LOG") == "" {
		claudemon.Env = []string{"RUST_LOG=claudemon=info"}
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
	hub := childSpec{Name: "hub", Bin: opts.HubBin, Args: hubArgs}

	adv := opts.AdvertiseHost
	if adv == "" {
		adv = opts.Host
	}
	hubHostPort := net.JoinHostPort(adv, fmt.Sprintf("%d", opts.HubPort))
	q := "?token=" + url.QueryEscape(opts.Token)

	return servePlan{
		Claudemon:       claudemon,
		Hub:             hub,
		ClaudemonHealth: apiURL + "/health",
		HubHealth:       fmt.Sprintf("http://127.0.0.1:%d/health", opts.HubPort),
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
		{"token", b.Token + "  (pairing credential — anyone who has it controls this server)"},
	}
	for _, r := range rows {
		fmt.Fprintf(&sb, "  %-10s %s\n", r[0], r[1])
	}
	sb.WriteString("\nPress Ctrl-C to stop.\n")
	return sb.String()
}
