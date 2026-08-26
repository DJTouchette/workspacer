package authtoken

import (
	"errors"
	"regexp"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// THE TIER, HELD TO THE RUNBOOKS THAT MINT IT.
//
// A tier that nothing is minted at is a tier that does not exist. `provider` is
// additive on purpose — the deployed node's operator token keeps working until
// somebody deliberately revokes it — which means the ONLY thing that actually
// moves a node off host authority is the line in the runbook an operator types.
// There is no code path that notices, no migration, and no warning: a runbook
// that says `--scope operator` deploys a node with nine authorities and nothing
// anywhere reports a problem.
//
// So the runbook line is the deliverable, and it is pinned like one. Same shape
// as TestTheDeployedNodeIDMatchesTheRegistryTheRunbooksWrite (internal/nodes),
// which holds fly.toml's WKS_NODE_ID against the nodes.json those same runbooks
// tell you to write.
func TestTheRunbooksMintTheNodeAProviderToken(t *testing.T) {
	// `workspacer token create --label fly-node --scope <tier>`, in either
	// flag order.
	mint := regexp.MustCompile(`workspacer token create[^\n]*--scope\s+([a-z]+)`)

	for _, runbook := range [][]string{
		{"deploy", "fly", "RUNBOOK.md"},
		{"deploy", "fly", "node", "RUNBOOK.md"},
	} {
		path := strings.Join(runbook, "/")
		body := readRepoFileForTest(t, runbook...)
		found := mint.FindAllStringSubmatch(body, -1)
		if len(found) == 0 {
			t.Errorf("%s no longer shows a `workspacer token create --scope …` for the node — this guard has nothing to check, and the node's tier is now whatever the reader guesses", path)
			continue
		}
		sawNodeMint := false
		for _, f := range found {
			line := f[0]
			if !strings.Contains(line, "fly-node") {
				continue // a phone/dashboard example, not the node's credential
			}
			sawNodeMint = true
			if Scope(f[1]) != ScopeProvider {
				t.Errorf("%s mints the node's token at `--scope %s`. That tier is TRUSTED on the bus — it may call nodes.wake (which spends money), nodes.sleep (which ends work in flight on a machine somebody is typing at), agents.spawn, config.save and jobs.upsert, it passes the HTTP token guard on POST /plugins/install (clone a repo and run its build step ON THE HUB), it may forge any host-owned topic, and it consumes the whole event firehose including raw PTY bytes for sessions on other machines. The node uses ONE of those. Mint --scope %s.",
					path, f[1], ScopeProvider)
			}
		}
		if !sawNodeMint {
			t.Errorf("%s has `workspacer token create --scope …` lines but none labelled fly-node — either the label changed (update this guard) or the node's mint instruction was dropped", path)
		}
	}
}

// readRepoFileForTest reads a file addressed from the monorepo root, through
// sweepguard for the reason internal/bus/policy_test.go documents: deploy/ is
// four levels ABOVE this module and cmd/go drops every test input whose path
// fails search.InDir, so a straight os.ReadFile would let this report
// `ok (cached)` after a runbook changed.
func readRepoFileForTest(t *testing.T, parts ...string) string {
	t.Helper()
	body, err := sweepguard.ReadRepoFile(parts...)
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout: %v", err)
		}
		t.Fatalf("%v", err)
	}
	return string(body)
}
