package main

import (
	"flag"
	"os"
	"path/filepath"
	"testing"
)

// The headless half of usage.pollOnBoot. Two claims, and the second is the one
// that matters: a setting that never reaches claudemon's environment is a
// setting the operator can toggle and watch do nothing.

func TestUsagePollOnBootSetting(t *testing.T) {
	tests := []struct {
		name string
		yaml string // "" = write no file at all
		want *bool
	}{
		{name: "no config file at all", yaml: "", want: nil},
		{name: "config with no usage section", yaml: "ui:\n  theme: everforest\n", want: nil},
		{name: "usage section with no key", yaml: "usage: {}\n", want: nil},
		{name: "explicitly off", yaml: "usage:\n  pollOnBoot: false\n", want: boolp(false)},
		{name: "explicitly on", yaml: "usage:\n  pollOnBoot: true\n", want: boolp(true)},
		// Unstated, NOT false: a document we cannot read says nothing about the
		// setting, and answering false would switch a feature off over a typo.
		{name: "unparseable document", yaml: "usage:\n\tpollOnBoot: false\n", want: nil},
		{name: "wrong type", yaml: "usage:\n  pollOnBoot: \"false\"\n", want: nil},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			if tc.yaml != "" {
				if err := os.WriteFile(filepath.Join(dir, "config.yaml"), []byte(tc.yaml), 0o600); err != nil {
					t.Fatal(err)
				}
			}
			got := usagePollOnBootSetting(dir)
			switch {
			case tc.want == nil && got != nil:
				t.Fatalf("got %v, want unstated (nil)", *got)
			case tc.want != nil && got == nil:
				t.Fatalf("got unstated (nil), want %v", *tc.want)
			case tc.want != nil && *got != *tc.want:
				t.Fatalf("got %v, want %v", *got, *tc.want)
			}
		})
	}
}

// The value has to arrive. buildServePlan is the only place `workspacer serve`
// can hand claudemon anything, so this is the whole headless delivery path.
func TestServePlanCarriesUsagePollOnBoot(t *testing.T) {
	base := serveOptions{
		Host: "127.0.0.1", HubPort: 7895, APIPort: 7891, HookPort: 7890,
		Token: "tok", ClaudemonBin: "/bin/claudemon", HubBin: "/bin/hub",
	}

	// Unstated: nothing set. claudemon reads an absent variable as ON, which is
	// the shipped default, so this must not spell it out as 0.
	if got := envValue(buildServePlan(base).Claudemon.Env, "WORKSPACER_USAGE_POLL_ON_BOOT"); got != "" {
		t.Fatalf("an unstated setting reached claudemon as %q; it must be unset", got)
	}

	off := base
	off.UsagePollOnBoot = boolp(false)
	if got := envValue(buildServePlan(off).Claudemon.Env, "WORKSPACER_USAGE_POLL_ON_BOOT"); got != "0" {
		t.Fatalf("pollOnBoot=false reached claudemon as %q, want 0", got)
	}

	on := base
	on.UsagePollOnBoot = boolp(true)
	if got := envValue(buildServePlan(on).Claudemon.Env, "WORKSPACER_USAGE_POLL_ON_BOOT"); got != "1" {
		t.Fatalf("pollOnBoot=true reached claudemon as %q, want 1", got)
	}

	// It rides ALONGSIDE the log level rather than replacing the slice — the
	// two are set by separate branches over one Env field.
	if got := envValue(buildServePlan(off).Claudemon.Env, "RUST_LOG"); os.Getenv("RUST_LOG") == "" && got == "" {
		t.Error("adding the usage variable dropped RUST_LOG from claudemon's environment")
	}
}

// The wiring between the two: `serve` must actually READ the config, not just
// be able to. This is the line that turns a config key into a spawn decision,
// and it has no other coverage — a serveOptions literal in a plan test proves
// only that buildServePlan would honour a value nobody supplied.
func TestResolveOptionsReadsUsagePollOnBoot(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	if err := os.MkdirAll(filepath.Join(dir, "workspacer"), 0o700); err != nil {
		t.Fatal(err)
	}
	write := func(body string) {
		if err := os.WriteFile(filepath.Join(dir, "workspacer", "config.yaml"), []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	resolve := func() serveOptions {
		fs := flag.NewFlagSet("serve", flag.ContinueOnError)
		cf := registerCommonServeFlags(fs)
		// Explicit binaries + token so resolution touches neither PATH nor the
		// token file: this test is about one field.
		if err := fs.Parse([]string{
			"-token", "tok", "-claudemon-bin", "/bin/claudemon", "-hub-bin", "/bin/hub",
		}); err != nil {
			t.Fatal(err)
		}
		opts, ok := cf.resolveOptions()
		if !ok {
			t.Fatal("resolveOptions refused a fully specified serve invocation")
		}
		return opts
	}

	write("usage:\n  pollOnBoot: false\n")
	off := resolve()
	if off.UsagePollOnBoot == nil || *off.UsagePollOnBoot {
		t.Fatalf("serve did not read usage.pollOnBoot: false from config.yaml (got %v)", off.UsagePollOnBoot)
	}
	// …and it reaches the daemon's environment from there.
	if got := envValue(buildServePlan(off).Claudemon.Env, "WORKSPACER_USAGE_POLL_ON_BOOT"); got != "0" {
		t.Fatalf("the config'd setting reached claudemon as %q, want 0", got)
	}

	write("ui:\n  theme: everforest\n")
	if quiet := resolve(); quiet.UsagePollOnBoot != nil {
		t.Fatalf("a config with no usage section resolved to %v, want unstated", *quiet.UsagePollOnBoot)
	}
}

func boolp(v bool) *bool { return &v }

// envValue reads NAME=value out of a childSpec Env slice.
func envValue(env []string, name string) string {
	for _, e := range env {
		if len(e) > len(name) && e[:len(name)] == name && e[len(name)] == '=' {
			return e[len(name)+1:]
		}
	}
	return ""
}
