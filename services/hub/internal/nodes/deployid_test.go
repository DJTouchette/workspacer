package nodes

import (
	"errors"
	"regexp"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// THE MITIGATION, HELD TO THE FILE IT MITIGATES.
//
// watchWake dropped its anonymous shortcut: a wake now completes only when the
// answering brain is attributed to the node being woken, and `attribute` will
// not guess between several registered nodes. The stated fix for the
// multi-node case is WKS_NODE_ID, which deploy/fly/node/fly.toml's [env] sets —
// and that value is only worth anything if it MATCHES the `id` of this
// machine's row in the hub's nodes.json. `attribute` ignores a name it does not
// recognise (it names a row the hub already holds and grants nothing, so
// trusting it would be worse), which means a mismatch degrades silently back to
// the anonymous behaviour the shortcut used to paper over.
//
// Silently, and on a SINGLE-NODE hub invisibly: `attribute("")` answers the one
// node, so everything works right up until a second node is registered and then
// NEITHER can be woken. The runbooks are the only place nodes.json is ever
// authored, so they are the other half of the pair, and this holds the three
// strings together the way TestTheTwoRestartPolicySpellingsAreBothRecorded
// holds the restart-policy pair.
func TestTheDeployedNodeIDMatchesTheRegistryTheRunbooksWrite(t *testing.T) {
	toml := readRepoFileForTest(t, "deploy", "fly", "node", "fly.toml")

	m := regexp.MustCompile(`(?m)^\s*WKS_NODE_ID\s*=\s*"([^"]*)"`).FindStringSubmatch(toml)
	if m == nil {
		t.Fatal("deploy/fly/node/fly.toml no longer sets WKS_NODE_ID — without it a deployed brain answers ANONYMOUSLY, and watchWake refuses to settle a wake it cannot attribute, so a second node in the registry makes every wake time out and stop the machine again")
	}
	nodeID := m[1]
	if nodeID == "" {
		t.Fatal("WKS_NODE_ID is set to the empty string, which brain.info drops entirely (it TrimSpaces and omits) — that is the anonymous case wearing a name")
	}

	// Both runbooks write nodes.json by heredoc, and `"id"` appears nowhere
	// else in either file. Every occurrence must therefore be this node's.
	idLine := regexp.MustCompile(`"id"\s*:\s*"([^"]*)"`)
	for _, runbook := range [][]string{
		{"deploy", "fly", "RUNBOOK.md"},
		{"deploy", "fly", "hub", "RUNBOOK.md"},
	} {
		path := strings.Join(runbook, "/")
		body := readRepoFileForTest(t, runbook...)
		found := idLine.FindAllStringSubmatch(body, -1)
		if len(found) == 0 {
			t.Errorf("%s no longer shows an `\"id\"` in the nodes.json it tells an operator to write — this guard has nothing to compare WKS_NODE_ID against", path)
			continue
		}
		for _, f := range found {
			if f[1] != nodeID {
				t.Errorf("%s tells an operator to register the node as %q and deploy/fly/node/fly.toml deploys it as WKS_NODE_ID=%q — the hub will not recognise that name, will log it and fall back to attributing anonymously, and the day a second node is registered neither one can be woken",
					path, f[1], nodeID)
			}
		}
	}
}

// readRepoFileForTest reads a file addressed from the monorepo root, through
// sweepguard for the reason internal/bus/policy_test.go documents: deploy/ is
// four levels ABOVE this module and cmd/go drops every test input whose path
// fails search.InDir, so a straight os.ReadFile would let this report
// `ok (cached)` after fly.toml changed.
func readRepoFileForTest(t *testing.T, parts ...string) string {
	t.Helper()
	raw, err := sweepguard.ReadRepoFile(parts...)
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout, so the deployment files are not here to cross-check: %v", err)
		}
		t.Fatalf("%v", err)
	}
	return string(raw)
}
