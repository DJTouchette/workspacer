package main

// The fixed terminal escalation contract, headless twin of
// apps/desktop/src/main/shared/workerEscalation.ts. Unlike resultSchema this is
// host-authored, so the brain can validate it exactly without implementing a
// caller-authored JSON Schema engine. It is injected only for a parented,
// non-manager fleet worker (spawnParams.isFleetWorker).

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"regexp"
	"strings"
)

const (
	workerEscalationFence    = "wks-escalation"
	workerEscalationMax      = 4096
	workerEscalationContract = "STRUCTURED WORKER ESCALATION CONTRACT. If you cannot safely complete the task because " +
		"you lack authority or need a manager/user decision, do not only refuse in prose. Stop " +
		"and write a concise explanation first, then END your final message with exactly one " +
		"fenced `wks-escalation` JSON block in this exact shape:\n\n```wks-escalation\n{\n" +
		"  \"type\": \"worker-escalation\",\n" +
		"  \"status\": \"blocked\",\n" +
		"  \"reason\": \"concise blocker\",\n" +
		"  \"requiredAuthorityOrDecision\": \"specific authority or decision needed\",\n" +
		"  \"changed\": false,\n" +
		"  \"nextAction\": \"useful next action for the manager\"\n" +
		"}\n```\n\nUse this only as a terminal escalation, not for a successful completion or a routine " +
		"progress update. Report truthfully whether anything changed. If a separate `wks-result` " +
		"contract is also present, emit `wks-result` when you complete the task; when you escalate, " +
		"emit `wks-escalation` instead. Emit the chosen terminal block only once and put nothing after it."
)

type workerEscalation struct {
	Type                        string `json:"type"`
	Status                      string `json:"status"`
	Reason                      string `json:"reason"`
	RequiredAuthorityOrDecision string `json:"requiredAuthorityOrDecision"`
	Changed                     *bool  `json:"changed"`
	NextAction                  string `json:"nextAction"`
}

type workerEscalationOutcome struct {
	JSON  string
	Error string
}

var workerEscalationBlockRE = regexp.MustCompile("(?is)```[ \\t]*wks-escalation[ \\t]*\\r?\\n(.*?)```")

// readWorkerEscalation returns nil when no tagged block exists. A non-nil
// outcome with Error means the worker attempted escalation but the host
// rejected it; callers preserve that error on an ordinary completion wake.
func readWorkerEscalation(finalMessage string) *workerEscalationOutcome {
	matches := workerEscalationBlockRE.FindAllStringSubmatch(finalMessage, -1)
	if len(matches) == 0 {
		return nil
	}
	block := matches[len(matches)-1][1]
	if len(block) > workerEscalationMax {
		return &workerEscalationOutcome{Error: fmt.Sprintf("the `%s` block is %d bytes; the limit is %d", workerEscalationFence, len(block), workerEscalationMax)}
	}

	var value workerEscalation
	dec := json.NewDecoder(bytes.NewBufferString(block))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&value); err != nil {
		return &workerEscalationOutcome{Error: fmt.Sprintf("the `%s` block is not valid escalation JSON: %v", workerEscalationFence, err)}
	}
	if err := dec.Decode(&struct{}{}); err != io.EOF {
		return &workerEscalationOutcome{Error: fmt.Sprintf("the `%s` block must contain one JSON object", workerEscalationFence)}
	}
	if value.Type != "worker-escalation" {
		return &workerEscalationOutcome{Error: `type: expected "worker-escalation"`}
	}
	if value.Status != "blocked" {
		return &workerEscalationOutcome{Error: `status: expected "blocked"`}
	}
	for key, text := range map[string]string{
		"reason":                      value.Reason,
		"requiredAuthorityOrDecision": value.RequiredAuthorityOrDecision,
		"nextAction":                  value.NextAction,
	} {
		if strings.TrimSpace(text) == "" {
			return &workerEscalationOutcome{Error: key + ": expected a non-empty string"}
		}
	}
	if value.Changed == nil {
		return &workerEscalationOutcome{Error: "changed: expected boolean"}
	}

	normalized := struct {
		Type                        string `json:"type"`
		Status                      string `json:"status"`
		Reason                      string `json:"reason"`
		RequiredAuthorityOrDecision string `json:"requiredAuthorityOrDecision"`
		Changed                     bool   `json:"changed"`
		NextAction                  string `json:"nextAction"`
	}{
		Type: "worker-escalation", Status: "blocked",
		Reason:                      strings.TrimSpace(value.Reason),
		RequiredAuthorityOrDecision: strings.TrimSpace(value.RequiredAuthorityOrDecision),
		Changed:                     *value.Changed, NextAction: strings.TrimSpace(value.NextAction),
	}
	raw, err := json.MarshalIndent(normalized, "", "  ")
	if err != nil {
		return &workerEscalationOutcome{Error: "the escalation could not be re-serialized"}
	}
	return &workerEscalationOutcome{JSON: string(raw)}
}
