package redact

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"testing"
)

// The token shape this leaked in production: 32 hex chars from the hub's
// minted remote-token.
const liveToken = "9f3c1ab27d5e40b8a6c2f10d4e7b83aa"

// TestDialErrorNeverCarriesTheToken reproduces the exact line observed on the
// Fly node — coder/websocket's dial failure embeds the full request URL — and
// asserts the sanitizer removes the credential while keeping the line useful.
func TestDialErrorNeverCarriesTheToken(t *testing.T) {
	dialURL := "https://workspacer-hub.tail65dbc5.ts.net/bus?token=" + liveToken
	// Verbatim shape of the coder/websocket error, context deadline and all.
	dialErr := fmt.Errorf("failed to WebSocket dial: failed to send handshake request: Get %q: %w", dialURL, context.DeadlineExceeded)

	line := fmt.Sprintf("brain: bus disconnected (%v); reconnecting in %s", Error(dialErr), "16s")

	if strings.Contains(line, liveToken) {
		t.Fatalf("token leaked into the log line: %s", line)
	}
	if !strings.Contains(line, "token="+Placeholder) {
		t.Fatalf("token param not redacted in place: %s", line)
	}
	for _, keep := range []string{"workspacer-hub.tail65dbc5.ts.net", "/bus", "context deadline exceeded", "reconnecting in 16s"} {
		if !strings.Contains(line, keep) {
			t.Fatalf("redaction destroyed diagnostics, %q missing: %s", keep, line)
		}
	}
	// The chain survives, so callers can still classify the failure.
	if !errors.Is(Error(dialErr), context.DeadlineExceeded) {
		t.Fatal("redacted error lost its wrapped cause")
	}
}

func TestErrorNilAndClean(t *testing.T) {
	if Error(nil) != nil {
		t.Fatal("Error(nil) must stay nil")
	}
	clean := errors.New("dial tcp 127.0.0.1:7895: connection refused")
	if got := Error(clean); got != clean {
		t.Fatalf("clean error should pass through unchanged, got %v", got)
	}
}

func TestText(t *testing.T) {
	cases := []struct{ name, in, want string }{
		{"query token", "ws://h:7895/bus?token=abc123", "ws://h:7895/bus?token=" + Placeholder},
		{"not first param", "ws://h/bus?internal=k1&token=abc123", "ws://h/bus?internal=" + Placeholder + "&token=" + Placeholder},
		{"mcp t param", "http://h:7897/mcp?t=abc123", "http://h:7897/mcp?t=" + Placeholder},
		{"plugin busToken", "http://h/plugins/x/ui?busToken=abc123&pane=1", "http://h/plugins/x/ui?busToken=" + Placeholder + "&pane=1"},
		{"case insensitive", "ws://h/bus?TOKEN=abc123", "ws://h/bus?TOKEN=" + Placeholder},
		{"quoted in error", `Get "ws://h/bus?token=abc123": timeout`, `Get "ws://h/bus?token=` + Placeholder + `": timeout`},
		{"trailing prose", "dial ws://h/bus?token=abc123 failed", "dial ws://h/bus?token=" + Placeholder + " failed"},
		{"empty value", "ws://h/bus?token=", "ws://h/bus?token=" + Placeholder},
		{"no query", "ws://h:7895/bus", "ws://h:7895/bus"},
		{"untouched param", "http://h/x?page=2", "http://h/x?page=2"},
		{"empty", "", ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := Text(c.in); got != c.want {
				t.Fatalf("Text(%q) = %q, want %q", c.in, got, c.want)
			}
		})
	}
}

// Redacting twice must not corrupt an already-redacted line.
func TestTextIsIdempotent(t *testing.T) {
	once := Text("ws://h/bus?token=" + liveToken)
	if twice := Text(once); twice != once {
		t.Fatalf("not idempotent: %q -> %q", once, twice)
	}
}

// A token can appear in more than one URL in one message (a peer list, a retry
// chain); every occurrence must go.
func TestTextRedactsEveryOccurrence(t *testing.T) {
	in := "peers: ws://a/bus?token=" + liveToken + ", ws://b/bus?token=" + liveToken
	got := Text(in)
	if strings.Contains(got, liveToken) {
		t.Fatalf("a token survived: %s", got)
	}
	if n := strings.Count(got, Placeholder); n != 2 {
		t.Fatalf("want 2 redactions, got %d: %s", n, got)
	}
}
