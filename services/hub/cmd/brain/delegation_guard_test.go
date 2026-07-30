// Cross-provider drift guard for the catalog-delegation split.
//
// The desktop registers bus capabilities through one of two doors:
//
//	cat('m', …)                 — a no-op when the catalog is delegated to this
//	                              brain (the default). The brain must provide m.
//	registerCapability('m', …)  — main always provides m itself.
//
// Choosing the wrong door fails silently in the worst way: the method simply has
// no provider on the bus, every remote call errors at runtime, and nothing fails
// at build or test time. That has now happened twice — fs.readImage was shipped
// behind `cat` with no brain counterpart (thumbnails dead for every web/remote
// client), and the fs.* path confinement was written behind a door the default
// configuration never opens (arbitrary host reads/writes for any bus client).
//
// So: assert the two provider lists partition the surface, in both directions.
package main

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

var (
	catRe = regexp.MustCompile(`(?m)^\s*cat\(\s*'([a-zA-Z][\w.]*)'`)
	regRe = regexp.MustCompile(`(?m)^\s*registerCapability\(\s*'([a-zA-Z][\w.]*)'`)
)

func readDesktopCapabilities(t *testing.T) string {
	t.Helper()
	// cmd/brain → repo root is four levels up (services/hub/cmd/brain).
	src := filepath.Join("..", "..", "..", "..", "apps", "desktop", "src", "main", "services", "hubCapabilities.ts")
	data, err := os.ReadFile(src)
	if err != nil {
		t.Skipf("hubCapabilities.ts not reachable (%v); skipping cross-repo cross-check", err)
	}
	return string(data)
}

func names(re *regexp.Regexp, body string) []string {
	var out []string
	for _, m := range re.FindAllStringSubmatch(body, -1) {
		out = append(out, m[1])
	}
	return out
}

func brainMethodSet() map[string]bool {
	r := newRegistry(newClaudemonClient("http://127.0.0.1:1"))
	set := map[string]bool{}
	// methods() is the full surface; catalog scope registers a subset. Delegation
	// hands the catalog scope to the brain, so that is the set that must cover
	// every `cat` method.
	for _, m := range r.methodsForScope("catalog") {
		set[m] = true
	}
	return set
}

// Every method the desktop delegates (registers via `cat`) must be provided by
// this brain in catalog scope — otherwise it has no provider at all by default.
func TestEveryDelegatedCapabilityHasABrainProvider(t *testing.T) {
	body := readDesktopCapabilities(t)
	delegated := names(catRe, body)
	if len(delegated) == 0 {
		t.Fatal("parsed no cat(...) capability names — the registration syntax changed; update catRe")
	}
	brain := brainMethodSet()
	for _, m := range delegated {
		if !brain[m] {
			t.Errorf("hubCapabilities.ts delegates %q via cat(...) but the brain's catalog scope does not provide it — "+
				"under the default configuration this method has NO provider on the bus. "+
				"Either implement it in the brain or register it with registerCapability.", m)
		}
	}
}

// The mirror image: a method main keeps for itself must NOT also be claimed by
// the brain's catalog scope. The router is single-owner per method, so two
// providers race and whichever registers first silently wins.
func TestMainOwnedCapabilitiesDoNotCollideWithTheBrain(t *testing.T) {
	body := readDesktopCapabilities(t)
	brain := brainMethodSet()
	// Methods main deliberately keeps even under delegation, because the brain's
	// version would be a degraded stand-in rather than a duplicate provider.
	allowedOverlap := map[string]bool{
		"notifications.post": true, // brain only logs it; main raises a real OS toast
	}
	for _, m := range names(regRe, body) {
		if brain[m] && !allowedOverlap[m] {
			t.Errorf("hubCapabilities.ts registers %q unconditionally while the brain's catalog scope also provides it — "+
				"two providers for one method; the router keeps whichever registered first.", m)
		}
	}
}

// fs.readImage is the concrete instance that shipped broken, so pin it by name:
// main must own it (the brain has no thumbnail implementation).
func TestFsReadImageIsOwnedByMainNotDelegated(t *testing.T) {
	body := readDesktopCapabilities(t)
	for _, m := range names(catRe, body) {
		if m == "fs.readImage" {
			t.Error("fs.readImage is delegated via cat(...) but no brain provider implements it — " +
				"thumbnails would fail for every web and remote client")
		}
	}
	if !strings.Contains(body, "registerCapability('fs.readImage'") {
		t.Error("fs.readImage should be registered with registerCapability so main provides it under delegation")
	}
}
