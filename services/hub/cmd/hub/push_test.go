package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/bus"
)

// writeTokens persists a scoped-token file the way `workspacer token create`
// does, so the store under test reads the real on-disk shape.
func writeTokens(t *testing.T, recs []authtoken.Record) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "tokens.json")
	b, err := json.Marshal(recs)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, b, 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

// A push subscription outlives the connection that made it, so revoking a
// device's token has to stop its notifications too — that is the whole reason
// the subscription records an identity.
func TestPushTokenValidatorFollowsRevocation(t *testing.T) {
	path := writeTokens(t, []authtoken.Record{{Token: "phone-tok", Scope: authtoken.ScopeTriage}})
	store := authtoken.NewStore(path)
	valid := pushTokenValidator("host-tok", store)

	phoneFP := bus.TokenFingerprint("phone-tok")
	if !valid(phoneFP) {
		t.Fatal("a live scoped token must keep its push subscription")
	}

	// Revoke by rewriting the file — the store re-reads on change, so this must
	// take effect with no hub restart.
	if err := os.WriteFile(path, []byte("[]"), 0o600); err != nil {
		t.Fatal(err)
	}
	if valid(phoneFP) {
		t.Fatal("a revoked token must stop being a push recipient")
	}
}

// The two credentials that are live but never appear in the scoped store. If
// either stopped validating, turning revocation on would silently cut push for
// every device that had ever paired — which is worse than the bug being fixed.
func TestPushTokenValidatorKeepsUnscopedCredentials(t *testing.T) {
	store := authtoken.NewStore(writeTokens(t, nil))

	// The host pairing token stays trusted rather than becoming a scoped record.
	if !pushTokenValidator("host-tok", store)(bus.TokenFingerprint("host-tok")) {
		t.Fatal("the host token must remain a valid push recipient")
	}

	// No recorded identity: a hub running with no token (the loopback default),
	// and every subscription written before identity was recorded.
	if !pushTokenValidator("host-tok", store)("") {
		t.Fatal("a subscription with no recorded identity must not be dropped")
	}
	if !pushTokenValidator("", nil)("") {
		t.Fatal("a tokenless hub must keep notifying its subscribers")
	}
}

// With no tokens file there is no scoped store at all; an unrecognized
// fingerprint must fail closed rather than nil-panic.
func TestPushTokenValidatorWithoutAStoreFailsClosed(t *testing.T) {
	if pushTokenValidator("host-tok", nil)(bus.TokenFingerprint("someone-else")) {
		t.Fatal("an unknown fingerprint must not be honored when there is no store")
	}
}
