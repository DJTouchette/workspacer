package capspec

import (
	"errors"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// mustReadRepoFile reads a cross-repo twin this package's guards compare
// against — apps/desktop/src/main/services/hubCapabilities.ts, the registration
// list every "is this capability path-scoped" test reads.
//
// The four readers here all t.Skipf'd on ANY read error, which means renaming
// or moving hubCapabilities.ts turned off four guards at once — including the
// one whose entire job is "a new path-bearing capability was added to the app
// but not scoped" — and printed `ok`. Only a genuinely absent checkout skips
// now; a missing file inside a present one fails. See sweepguard.
func mustReadRepoFile(t *testing.T, parts ...string) []byte {
	t.Helper()
	data, err := sweepguard.ReadRepoFile(parts...)
	if err == nil {
		return data
	}
	if errors.Is(err, sweepguard.ErrNoCheckout) {
		t.Skipf("not a monorepo checkout, so this cross-repo cross-check has nothing to read: %v", err)
	}
	t.Fatalf("%v", err)
	return nil
}

// desktopCapabilitiesSrc is the one twin this package reads, named once.
var desktopCapabilitiesSrc = []string{"apps", "desktop", "src", "main", "services", "hubCapabilities.ts"}
