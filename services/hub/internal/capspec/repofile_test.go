package capspec

import (
	"bytes"
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
		// Normalize line endings. Every scanner in this package delimits source
		// TEXTUALLY — a function body ends at "\n}\n", a handler's extent is
		// found by searching for it — and GitHub's Windows runners check the repo
		// out with CRLF, where none of those needles match. The measured
		// consequence was not a clean failure: the delimiter silently ran to EOF,
		// so /health's handler "body" swallowed a LATER function's 401 and the
		// route was classified guarded on Windows and unguarded on Linux, from
		// the same source. A scanner that reads code has no business caring how
		// the checkout spells a newline.
		return bytes.ReplaceAll(data, []byte("\r\n"), []byte("\n"))
	}
	if errors.Is(err, sweepguard.ErrNoCheckout) {
		t.Skipf("not a monorepo checkout, so this cross-repo cross-check has nothing to read: %v", err)
	}
	t.Fatalf("%v", err)
	return nil
}

// desktopCapabilitiesSrc is the one twin this package reads, named once.
var desktopCapabilitiesSrc = []string{"apps", "desktop", "src", "main", "services", "hubCapabilities.ts"}
