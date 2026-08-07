package main

import (
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

// captureStdout runs fn with os.Stdout redirected and returns what it printed.
func captureStdout(t *testing.T, fn func()) string {
	t.Helper()
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	orig := os.Stdout
	os.Stdout = w
	defer func() { os.Stdout = orig }()
	fn()
	w.Close()
	out, err := io.ReadAll(r)
	if err != nil {
		t.Fatal(err)
	}
	return string(out)
}

func TestTokenCreateListRevoke(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")

	// create: prints the bare token on stdout (script-friendly) and persists it.
	out := captureStdout(t, func() {
		if code := runTokenCreate([]string{"--scope", "triage", "--label", "phone", "--tokens-file", path}); code != 0 {
			t.Errorf("create exit = %d", code)
		}
	})
	tok := strings.TrimSpace(out)
	if len(tok) < 20 {
		t.Fatalf("created token output = %q, want the bare token", out)
	}
	recs, err := authtoken.Load(path)
	if err != nil || len(recs) != 1 {
		t.Fatalf("persisted records = %+v (%v), want exactly the minted one", recs, err)
	}
	if recs[0].Token != tok || recs[0].Scope != authtoken.ScopeTriage || recs[0].Label != "phone" {
		t.Fatalf("persisted record = %+v, want scope triage, label phone, token %q", recs[0], tok)
	}

	// list shows it with its scope + label.
	out = captureStdout(t, func() {
		if code := runTokenList([]string{"--tokens-file", path}); code != 0 {
			t.Errorf("list exit = %d", code)
		}
	})
	if !strings.Contains(out, tok) || !strings.Contains(out, "triage") || !strings.Contains(out, "phone") {
		t.Fatalf("list output = %q, want the token + scope + label", out)
	}

	// revoke by prefix empties the store.
	if code := runTokenRevoke([]string{"--tokens-file", path, tok[:12]}); code != 0 {
		t.Fatalf("revoke exit = %d", code)
	}
	recs, _ = authtoken.Load(path)
	if len(recs) != 0 {
		t.Fatalf("records after revoke = %+v, want none", recs)
	}
}

func TestTokenCreateValidation(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	if code := runTokenCreate([]string{"--tokens-file", path}); code == 0 {
		t.Error("create without --scope must fail")
	}
	if code := runTokenCreate([]string{"--scope", "root", "--tokens-file", path}); code == 0 {
		t.Error("create with an unknown scope must fail")
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Error("failed creates must not write the tokens file")
	}
}

func TestTokenRevokeUnknown(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")
	if code := runTokenRevoke([]string{"--tokens-file", path, "does-not-exist"}); code == 0 {
		t.Error("revoking an unknown token must fail loudly")
	}
	if code := runTokenRevoke([]string{"--tokens-file", path}); code == 0 {
		t.Error("revoke with no argument must fail")
	}
}

// authtoken mints with base64.RawURLEncoding, whose alphabet includes '-', so
// roughly one token in 64 starts with one — and `flag` reads a bare
// "-QWGMC1Ib9FK" as an unknown flag. With flag.ExitOnError that took the whole
// process down (inside `go test`, the test binary) and revoked nothing, which
// means the only documented way to invalidate a leaked bus credential did not
// work for exactly the credentials whose spelling made it matter. The existing
// round-trip test passes tok[:12] positionally, so it flaked at that rate too.
//
// Deterministic: mint until one comes out with a leading dash rather than hoping.
func TestTokenRevokeAcceptsATokenThatStartsWithADash(t *testing.T) {
	path := filepath.Join(t.TempDir(), "tokens.json")

	// mintDashed returns a freshly minted token whose spelling starts with '-',
	// or "" if 2000 tries did not produce one (astronomically unlikely at 1/64).
	mintDashed := func() string {
		for i := 0; i < 2000; i++ {
			rec, err := authtoken.Mint(path, authtoken.ScopeView, "probe")
			if err != nil {
				t.Fatal(err)
			}
			if strings.HasPrefix(rec.Token, "-") {
				return rec.Token
			}
		}
		return ""
	}

	// Both the full token and the >=8-char prefix `workspacer token list` prints,
	// and both spellings of the one flag this command declares.
	for _, form := range []struct {
		name string
		args func(tok string) []string
	}{
		{"full token", func(tok string) []string { return []string{"--tokens-file", path, tok} }},
		{"listed prefix", func(tok string) []string { return []string{"--tokens-file", path, tok[:12]} }},
		{"--flag=value form", func(tok string) []string { return []string{"--tokens-file=" + path, tok} }},
		// (No "token before the flag" form: Go's flag package stops parsing at
		// the first non-flag argument, so flags-after-positionals never worked
		// for any subcommand here and is not what this fix is about.)
	} {
		t.Run(form.name, func(t *testing.T) {
			tok := mintDashed()
			if tok == "" {
				t.Skip("no dash-leading token in 2000 mints")
			}
			before, _ := authtoken.Load(path)
			if code := runTokenRevoke(form.args(tok)); code != 0 {
				t.Fatalf("revoke exited %d for %q; a bus token is not a flag just because base64url gave it a leading '-'", code, tok)
			}
			after, _ := authtoken.Load(path)
			if len(after) != len(before)-1 {
				t.Fatalf("revoke removed %d records, want exactly 1", len(before)-len(after))
			}
			for _, r := range after {
				if r.Token == tok {
					t.Fatalf("%q is still live after being revoked", tok)
				}
			}
		})
	}
}
