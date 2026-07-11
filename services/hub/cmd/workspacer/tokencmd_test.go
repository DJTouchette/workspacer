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
