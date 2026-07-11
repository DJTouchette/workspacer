package main

import (
	"encoding/json"
	"strings"
	"testing"
)

// argsAfter returns the value following flag in args, or "".
func argsAfter(args []string, flag string) string {
	for i, a := range args {
		if a == flag && i+1 < len(args) {
			return args[i+1]
		}
	}
	return ""
}

func hasFlag(args []string, flag string) bool {
	for _, a := range args {
		if a == flag {
			return true
		}
	}
	return false
}

func TestBuildServePlanWiring(t *testing.T) {
	base := serveOptions{
		Host: "127.0.0.1", HubPort: 7895, APIPort: 7891, HookPort: 7890,
		Token: "tok", ClaudemonBin: "/bin/claudemon", HubBin: "/bin/hub",
	}
	tests := []struct {
		name string
		opts serveOptions
		want func(t *testing.T, p servePlan)
	}{
		{
			name: "defaults wire claudemon loopback ports into the hub argv",
			opts: base,
			want: func(t *testing.T, p servePlan) {
				if got := argsAfter(p.Claudemon.Args, "--api-port"); got != "7891" {
					t.Errorf("claudemon --api-port = %q, want 7891", got)
				}
				if got := argsAfter(p.Claudemon.Args, "--host"); got != "127.0.0.1" {
					t.Errorf("claudemon --host = %q, want loopback", got)
				}
				if got := argsAfter(p.Hub.Args, "--claudemon"); got != "http://127.0.0.1:7891" {
					t.Errorf("hub --claudemon = %q", got)
				}
				if got := argsAfter(p.Hub.Args, "--claudemon-events"); got != "http://127.0.0.1:7891/events" {
					t.Errorf("hub --claudemon-events = %q", got)
				}
				if got := argsAfter(p.Hub.Args, "--brain-scope"); got != "full" {
					t.Errorf("hub --brain-scope = %q, want full (a headless server needs the whole surface)", got)
				}
				if got := argsAfter(p.Hub.Args, "--token"); got != "tok" {
					t.Errorf("hub --token = %q", got)
				}
				if hasFlag(p.Hub.Args, "--brain-bin") || hasFlag(p.Hub.Args, "--plugins-dir") || hasFlag(p.Hub.Args, "--webapp-dir") {
					t.Errorf("optional flags leaked into hub argv: %v", p.Hub.Args)
				}
			},
		},
		{
			name: "custom ports flow into argv, health URLs, and banner",
			opts: func() serveOptions {
				o := base
				o.HubPort, o.APIPort, o.HookPort = 18895, 18891, 18890
				return o
			}(),
			want: func(t *testing.T, p servePlan) {
				if got := argsAfter(p.Hub.Args, "--addr"); got != "127.0.0.1:18895" {
					t.Errorf("hub --addr = %q", got)
				}
				if got := argsAfter(p.Claudemon.Args, "--hook-port"); got != "18890" {
					t.Errorf("claudemon --hook-port = %q", got)
				}
				if p.ClaudemonHealth != "http://127.0.0.1:18891/health" {
					t.Errorf("ClaudemonHealth = %q", p.ClaudemonHealth)
				}
				if p.HubHealth != "http://127.0.0.1:18895/health" {
					t.Errorf("HubHealth = %q", p.HubHealth)
				}
				if p.Banner.BusURL != "ws://127.0.0.1:18895/bus" {
					t.Errorf("BusURL = %q", p.Banner.BusURL)
				}
			},
		},
		{
			name: "wide bind advertises the given host but health stays loopback",
			opts: func() serveOptions {
				o := base
				o.Host, o.AdvertiseHost = "0.0.0.0", "100.100.1.2"
				return o
			}(),
			want: func(t *testing.T, p servePlan) {
				if got := argsAfter(p.Hub.Args, "--addr"); got != "0.0.0.0:7895" {
					t.Errorf("hub --addr = %q", got)
				}
				if p.Banner.RemoteURL != "http://100.100.1.2:7895/remote?token=tok" {
					t.Errorf("RemoteURL = %q", p.Banner.RemoteURL)
				}
				if !strings.HasPrefix(p.HubHealth, "http://127.0.0.1:") {
					t.Errorf("HubHealth should probe loopback, got %q", p.HubHealth)
				}
			},
		},
		{
			name: "optional brain/plugins/webapp paths ride the hub argv",
			opts: func() serveOptions {
				o := base
				o.BrainBin, o.PluginsDir, o.WebappDir = "/opt/brain", "/tmp/plugins", "/tmp/web"
				return o
			}(),
			want: func(t *testing.T, p servePlan) {
				if got := argsAfter(p.Hub.Args, "--brain-bin"); got != "/opt/brain" {
					t.Errorf("hub --brain-bin = %q", got)
				}
				if got := argsAfter(p.Hub.Args, "--plugins-dir"); got != "/tmp/plugins" {
					t.Errorf("hub --plugins-dir = %q", got)
				}
				if got := argsAfter(p.Hub.Args, "--webapp-dir"); got != "/tmp/web" {
					t.Errorf("hub --webapp-dir = %q", got)
				}
			},
		},
		{
			name: "token is URL-escaped in the pairing links",
			opts: func() serveOptions {
				o := base
				o.Token = "a+b/c"
				return o
			}(),
			want: func(t *testing.T, p servePlan) {
				if !strings.HasSuffix(p.Banner.MobileURL, "/m?token=a%2Bb%2Fc") {
					t.Errorf("MobileURL not escaped: %q", p.Banner.MobileURL)
				}
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			tt.want(t, buildServePlan(tt.opts))
		})
	}
}

func TestAdvertiseHost(t *testing.T) {
	tests := []struct {
		name  string
		bind  string
		ipv4s []string
		want  string
	}{
		{"concrete host wins", "192.168.1.5", []string{"100.100.0.1"}, "192.168.1.5"},
		{"loopback stays loopback", "127.0.0.1", []string{"100.100.0.1"}, "127.0.0.1"},
		{"wildcard prefers tailscale CGNAT", "0.0.0.0", []string{"192.168.1.5", "100.100.0.1"}, "100.100.0.1"},
		{"public 100.0.x.x is not tailscale", "0.0.0.0", []string{"100.0.0.1", "192.168.1.5"}, "100.0.0.1"},
		{"wildcard falls back to first ipv4", "0.0.0.0", []string{"192.168.1.5", "10.0.0.2"}, "192.168.1.5"},
		{"wildcard with no addresses falls back to loopback", "0.0.0.0", nil, "127.0.0.1"},
		{"ipv6 wildcard treated like 0.0.0.0", "::", []string{"100.99.1.1"}, "100.99.1.1"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := advertiseHost(tt.bind, tt.ipv4s); got != tt.want {
				t.Errorf("advertiseHost(%q, %v) = %q, want %q", tt.bind, tt.ipv4s, got, tt.want)
			}
		})
	}
}

func TestBannerShapes(t *testing.T) {
	b := bannerInfo{
		BusURL:       "ws://127.0.0.1:7895/bus",
		RemoteURL:    "http://127.0.0.1:7895/remote?token=t",
		MobileURL:    "http://127.0.0.1:7895/m?token=t",
		HubURL:       "http://127.0.0.1:7895",
		ClaudemonURL: "http://127.0.0.1:7891",
		Token:        "t",
	}

	t.Run("human banner names every endpoint and labels the token", func(t *testing.T) {
		out := renderBanner(b)
		for _, want := range []string{b.BusURL, b.RemoteURL, b.MobileURL, b.ClaudemonURL, "pairing credential"} {
			if !strings.Contains(out, want) {
				t.Errorf("banner missing %q:\n%s", want, out)
			}
		}
	})

	t.Run("JSON round-trips with stable keys", func(t *testing.T) {
		raw, err := json.Marshal(b)
		if err != nil {
			t.Fatal(err)
		}
		var m map[string]string
		if err := json.Unmarshal(raw, &m); err != nil {
			t.Fatal(err)
		}
		for _, key := range []string{"busUrl", "remoteUrl", "mobileUrl", "hubUrl", "claudemonUrl", "token"} {
			if _, ok := m[key]; !ok {
				t.Errorf("JSON banner missing key %q: %s", key, raw)
			}
		}
	})
}
