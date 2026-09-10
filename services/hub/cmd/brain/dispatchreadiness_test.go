package main

import "testing"

func TestRemoteLoginReadinessDoesNotPromoteNegativeOrUnknownStatus(t *testing.T) {
	for _, tc := range []struct {
		provider, output string
		success          bool
		want             *bool
	}{
		{"codex", "Not logged in", true, boolPtr(false)},
		{"codex", "Not logged in", false, boolPtr(false)},
		{"codex", "Logged in using ChatGPT", true, boolPtr(true)},
		{"codex", "Logged in using ChatGPT", false, nil},
		{"claude", `{"loggedIn":true}`, true, boolPtr(true)},
		{"claude", `{"loggedIn":true}`, false, nil},
		{"claude", `{"loggedIn":false}`, true, boolPtr(false)},
		{"claude", `{"oauthAccount":{"email":"stale@example.invalid"}}`, true, nil},
		{"claude", "unrecognized command", false, nil},
	} {
		got := providerLoginFromOutput(tc.provider, []byte(tc.output), tc.success)
		if (got == nil) != (tc.want == nil) || (got != nil && tc.want != nil && *got != *tc.want) {
			t.Errorf("%s status %q (success=%v): got %v, want %v", tc.provider, tc.output, tc.success, got, tc.want)
		}
	}
}
