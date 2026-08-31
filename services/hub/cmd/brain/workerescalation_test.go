package main

import (
	"strings"
	"testing"
)

const validEscalationJSON = `{"type":"worker-escalation","status":"blocked","reason":"publishing requires write authority","requiredAuthorityOrDecision":"authorize a release-capable worker","changed":false,"nextAction":"review the artifact, then redispatch publishing"}`

func TestWorkerEscalationContractAndParser(t *testing.T) {
	if !strings.Contains(workerEscalationContract, "wks-escalation") ||
		!strings.Contains(workerEscalationContract, "requiredAuthorityOrDecision") ||
		!strings.Contains(workerEscalationContract, "wks-result") {
		t.Fatalf("contract does not advertise the fixed shape and result compatibility:\n%s", workerEscalationContract)
	}
	out := readWorkerEscalation("cannot publish\n\n```wks-escalation\n" + validEscalationJSON + "\n```")
	if out == nil || out.Error != "" || !strings.Contains(out.JSON, `"changed": false`) {
		t.Fatalf("valid escalation was not preserved: %+v", out)
	}
}

func TestWorkerEscalationMalformedAndAbsent(t *testing.T) {
	bad := readWorkerEscalation("```wks-escalation\n{\"type\":\"worker-escalation\"}\n```")
	if bad == nil || bad.Error == "" || bad.JSON != "" {
		t.Fatalf("malformed escalation was silently accepted: %+v", bad)
	}
	if got := readWorkerEscalation("I cannot publish because I only have read-only authority."); got != nil {
		t.Fatalf("ordinary prose refusal became an escalation: %+v", got)
	}
}
