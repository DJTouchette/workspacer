// Package redact strips credentials out of text that is about to be logged.
//
// THE LEAK THIS EXISTS FOR: a WebSocket/HTTP dial error embeds the request URL
// verbatim, and every bus dial in this repo carries its auth token as a
// `?token=` query param (a browser WS handshake cannot set an Authorization
// header, so the query form is the canonical one — see internal/bus
// presentedToken). So the ordinary "reconnecting in 16s" line the brain writes
// on every failed hub dial published the node's HUB_TOKEN in plaintext, once
// per attempt, into whatever collects the process's stderr — observed live on a
// Fly node, dozens of lines inside one two-minute hub outage.
//
// The rule this package encodes: a URL may be logged, a credential may not.
// Apply Text/Error at (or before) any log site that can carry a URL — the
// helpers are cheap, idempotent, and safe on text that holds no URL at all.
package redact

import (
	"regexp"
	"strings"
)

// Placeholder is what a credential value is replaced by.
const Placeholder = "REDACTED"

// credentialQuery matches a credential-bearing query parameter and its value
// anywhere in free text — the value runs to the next delimiter, so it survives
// a URL quoted inside a Go error string (`Get "wss://h/bus?token=abc": …`).
//
// The parameter names are the ones this codebase actually authenticates with:
//
//	token      the hub bus / HTTP token (internal/bus presentedToken)
//	busToken   the per-plugin or per-pane token in a webview URL
//	t          the MCP facade's query credential (cmd/mcp presentedToken)
//	internal   the hub's in-process dial key (internal/bus InternalDialURL)
//
// plus the conventional spellings a dependency or a future route might use.
// Matching a non-secret `t=`/`key=` param is harmless — the cost of over-
// redacting a log line is nothing next to the cost of printing a token.
var credentialQuery = regexp.MustCompile(
	`(?i)([?&](?:token|bus_?token|auth_?token|access_?token|api_?key|apikey|secret|password|passwd|pwd|key|sig|signature|internal|t)=)[^&\s"'\\<>)\]]*`)

// Text returns s with the value of every credential-bearing query parameter
// replaced by Placeholder. Everything else — scheme, host, path, the parameter
// name itself, the surrounding error prose — is preserved, so the line stays as
// diagnosable as it was.
func Text(s string) string {
	if s == "" || !strings.ContainsAny(s, "?&") {
		return s
	}
	return credentialQuery.ReplaceAllString(s, "${1}"+Placeholder)
}

// URL is Text named for its most common argument. A URL string is just text as
// far as this package is concerned; it is never parsed, so a malformed one is
// redacted just the same.
func URL(u string) string { return Text(u) }

// Error returns err with its message redacted, preserving the chain so
// errors.Is/errors.As keep working on the original.
//
// The redaction is applied by THIS error's Error(); the wrapped error still
// holds the raw text, which is the price of keeping Is/As intact. Callers must
// therefore log the value this returns — not errors.Unwrap of it.
func Error(err error) error {
	if err == nil {
		return nil
	}
	msg := err.Error()
	clean := Text(msg)
	if clean == msg {
		return err
	}
	return redactedError{err: err, msg: clean}
}

type redactedError struct {
	err error
	msg string
}

func (e redactedError) Error() string { return e.msg }
func (e redactedError) Unwrap() error { return e.err }
