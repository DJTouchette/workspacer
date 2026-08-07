// Package extinput makes `go test`'s result cache notice when a file OUTSIDE
// this Go module changes.
//
// Every cross-language guard in this repo reads something the Go module does
// not contain: capspec's detectors parse apps/desktop/src/main/services/
// hubCapabilities.ts, and the corpus loaders parse contracts/*.json. Both live
// above services/hub, and cmd/go's test cache says this about such files
// (cmd/go/internal/test/test.go, computeTestInputsID):
//
//	if a.Package.Root == "" || search.InDir(name, a.Package.Root) == "" {
//		// Do not recheck files outside the module, GOPATH, or GOROOT root.
//		break
//	}
//
// So the read is invisible to the cache key. Edit hubCapabilities.ts to add an
// unclassified `env` param to terminals.create, run `go test ./...`, and the
// detector that exists to catch exactly that prints `ok (cached)` without ever
// re-reading the file. Every fix pinned by a cross-repo guard is therefore
// pinned only until the next cached run — which is the failure mode this whole
// round is about: guards that report a pass they did not earn.
//
// The fix is to put the file's CONTENT into the cache key by a route the cache
// does record. Environment reads are recorded (`getenv <name>` in the testlog,
// hashed by name and value), so reading a variable whose NAME carries the
// file's content hash makes any edit to that file a different cache key, and
// therefore a cache miss. Nothing reads the variable's value; the name is the
// whole signal, and no such variable is ever set.
//
// Use [ReadFile] wherever a test reads a repo file from outside services/hub.
package extinput

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
)

// envPrefix names the synthetic variables. It is deliberately recognizable: if
// it ever shows up in a testlog dump or a debug trace, this comment is findable.
const envPrefix = "WKS_EXTERNAL_INPUT_"

// Pin records a content hash in the running test's cache key. Call it with the
// bytes of any input the Go module does not contain.
func Pin(data []byte) {
	sum := sha256.Sum256(data)
	os.Getenv(envPrefix + hex.EncodeToString(sum[:]))
}

// ReadFile is os.ReadFile plus the cache pin, for inputs outside the module.
//
// The ERROR path is pinned too, and it matters as much as the success path: a
// guard that skips when its cross-repo file is missing would otherwise cache
// that skip and keep reporting it after the file comes back — a guard that
// switched itself off and stayed off.
func ReadFile(path string) ([]byte, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		Pin([]byte(fmt.Sprintf("extinput-error %s: %v", path, err)))
		return nil, err
	}
	Pin(data)
	return data, nil
}
