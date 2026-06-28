package sandbox

import (
	"strings"
	"testing"
)

func TestParseMode(t *testing.T) {
	cases := map[string]Mode{
		"off":         ModeOff,
		"enforce":     ModeEnforce,
		"best-effort": ModeBestEffort,
		"":            ModeBestEffort,
		"garbage":     ModeBestEffort,
		"  enforce  ": ModeEnforce,
	}
	for in, want := range cases {
		if got := ParseMode(in); got != want {
			t.Errorf("ParseMode(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestDecide(t *testing.T) {
	cases := []struct {
		mode      Mode
		available bool
		want      Decision
	}{
		{ModeOff, true, RunUnsandboxed},
		{ModeOff, false, RunUnsandboxed},
		{ModeBestEffort, true, RunSandboxed},
		{ModeBestEffort, false, RunUnsandboxed},
		{ModeEnforce, true, RunSandboxed},
		{ModeEnforce, false, Refuse}, // fail closed
	}
	for _, c := range cases {
		if got := Decide(c.mode, c.available); got != c.want {
			t.Errorf("Decide(%q, %v) = %d, want %d", c.mode, c.available, got, c.want)
		}
	}
}

func TestBuildBwrapArgs(t *testing.T) {
	args := buildBwrapArgs("/usr/bin/python3", []string{"-m", "http.server"},
		Policy{WriteRoots: []string{"/plugins/acme", ""}}) // empty root skipped

	joined := strings.Join(args, " ")
	// Read-only host, then a rw re-bind of the plugin dir.
	if !strings.Contains(joined, "--ro-bind / /") {
		t.Errorf("missing read-only root bind: %q", joined)
	}
	if !strings.Contains(joined, "--bind /plugins/acme /plugins/acme") {
		t.Errorf("missing rw bind of the write root: %q", joined)
	}
	if !strings.Contains(joined, "--die-with-parent") {
		t.Errorf("missing --die-with-parent: %q", joined)
	}
	if strings.Contains(joined, "--bind  ") {
		t.Errorf("empty write root should be skipped, got %q", joined)
	}
	// The command + its args come after the `--` separator, in order.
	sep := indexOf(args, "--")
	if sep < 0 || sep+3 > len(args) {
		t.Fatalf("expected `-- python3 -m http.server` tail, got %v", args)
	}
	tail := args[sep+1:]
	want := []string{"/usr/bin/python3", "-m", "http.server"}
	for i, w := range want {
		if tail[i] != w {
			t.Fatalf("tail[%d] = %q, want %q (full: %v)", i, tail[i], w, tail)
		}
	}
}

func TestBuildSeatbeltProfile(t *testing.T) {
	profile := buildSeatbeltProfile(Policy{WriteRoots: []string{"/plugins/acme"}})

	for _, want := range []string{
		"(allow default)",
		"(deny file-write*)",
		`(subpath "/plugins/acme")`,
		`(subpath "/private/tmp")`,
	} {
		if !strings.Contains(profile, want) {
			t.Errorf("profile missing %q:\n%s", want, profile)
		}
	}
	// The blanket deny must come before the re-allows (last match wins in SBPL).
	if strings.Index(profile, "(deny file-write*)") > strings.Index(profile, `(subpath "/plugins/acme")`) {
		t.Errorf("deny must precede the write-root re-allow:\n%s", profile)
	}
}

func TestSbplStringEscaping(t *testing.T) {
	if got := sbplString(`/a/b"c\d`); got != `"/a/b\"c\\d"` {
		t.Errorf("sbplString escaping = %s", got)
	}
}

func TestWrapReturnsRunnableCommand(t *testing.T) {
	// Regardless of platform/mechanism availability, Wrap must yield a runnable
	// command — wrapped when available, the original otherwise.
	res := Wrap("/bin/echo", []string{"hi"}, Policy{WriteRoots: []string{"/tmp/x"}})
	if res.Path == "" {
		t.Fatal("Wrap returned empty Path")
	}
	if !res.Available && res.Path != "/bin/echo" {
		t.Errorf("unavailable Wrap should pass through the original command, got %q", res.Path)
	}
}

func indexOf(ss []string, target string) int {
	for i, s := range ss {
		if s == target {
			return i
		}
	}
	return -1
}
