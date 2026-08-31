package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// THE CLAMP. agents.spawn refuses to start a bypassing agent for a bus caller;
// claude.setPermissionMode reaches an agent that is ALREADY running and does no
// ownership check on the sessionId, so without the same refusal the spawn clamp
// is one extra call away from being irrelevant.
//
// The assertion that matters is not the error — it is that claudemon was NEVER
// CONTACTED. A refusal that still posted the mode and then reported failure
// would have already switched the agent.
func TestSetPermissionModeRefusesEveryEscalationBeforeItTravels(t *testing.T) {
	for _, mode := range []string{
		"bypassPermissions", // claude's spelling
		"yolo",              // codex/opencode/pi's spelling
		"auto",              // claudemon's stream endpoint accepts it; no menu shows it
		"dontAsk",
		"BYPASSPERMISSIONS", // the allowlist is exact — a case variant is unknown, so refused
		"acceptedits",       // and so is a lowercased spelling of an ALLOWED mode
	} {
		t.Run(mode, func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			reg := newRegistry(newClaudemonClient(srv.URL))

			params, _ := json.Marshal(map[string]any{"sessionId": "s1", "mode": mode})
			_, err := reg.handle(context.Background(), "claude.setPermissionMode", params)
			if err == nil {
				t.Fatalf("claude.setPermissionMode(%q) was ACCEPTED — a bus caller can turn the host's approvals off on a running agent", mode)
			}
			if !strings.Contains(err.Error(), "claude.setPermissionMode") {
				t.Errorf("refusal does not name the capability: %v", err)
			}
			if hits := rec.calls("/sessions/s1/permission-mode"); len(hits) != 0 {
				t.Fatalf("claudemon was contacted %d time(s) for a REFUSED mode — the switch already happened", len(hits))
			}
		})
	}
}

// The refusal is asymmetric on purpose: tightening is not an escalation, and a
// remote operator has to be able to put a runaway worker back into ask mode.
func TestSetPermissionModeAllowsTighteningAndNeutralModes(t *testing.T) {
	for _, mode := range []string{"default", "ask", "acceptEdits", "plan", "manual"} {
		t.Run(mode, func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			reg := newRegistry(newClaudemonClient(srv.URL))

			params, _ := json.Marshal(map[string]any{"sessionId": "s1", "mode": mode})
			res, err := reg.handle(context.Background(), "claude.setPermissionMode", params)
			if err != nil {
				t.Fatalf("mode %q was refused: %v", mode, err)
			}
			hits := rec.calls("/sessions/s1/permission-mode")
			if len(hits) != 1 {
				t.Fatalf("expected one POST to claudemon, got %d", len(hits))
			}
			// The CHECKED string is the one that travels.
			if hits[0].body["mode"] != mode {
				t.Errorf("claudemon received mode %v, want %q", hits[0].body["mode"], mode)
			}
			var out liveControlResult
			if err := json.Unmarshal(res, &out); err != nil {
				t.Fatal(err)
			}
			if !out.OK {
				t.Errorf("result not ok: %+v", out)
			}
		})
	}
}

// A daemon that cannot do the switch live is `{ok:false, error}` — NOT a
// transport error — so the client can offer the restart path. The desktop
// handler answers the same shape.
func TestSetPermissionModeReportsADaemonRefusalAsOkFalse(t *testing.T) {
	rec := newRecorder()
	rec.status["/sessions/s1/permission-mode"] = 409
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	res, err := reg.handle(context.Background(), "claude.setPermissionMode",
		json.RawMessage(`{"sessionId":"s1","mode":"plan"}`))
	if err != nil {
		t.Fatalf("a daemon 409 became a call error: %v", err)
	}
	var out liveControlResult
	_ = json.Unmarshal(res, &out)
	if out.OK || out.Error == "" {
		t.Fatalf("want ok:false with a reason, got %+v", out)
	}
}

// setEffort BRANCHES ON PROVIDER — the whole reason the desktop factored it
// into one shared body. A claude session takes `/effort <level>` through the
// queued message path (there is no set_effort in the stream control protocol);
// a managed one takes the structural /model endpoint.
func TestSetEffortBranchesOnProvider(t *testing.T) {
	t.Run("claude uses the message path", func(t *testing.T) {
		rec := newRecorder()
		srv := rec.server()
		defer srv.Close()
		reg := newRegistry(newClaudemonClient(srv.URL))
		reg.store = newSessionStore()
		reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude"}`))

		if _, err := reg.handle(context.Background(), "claude.setEffort",
			json.RawMessage(`{"sessionId":"s1","effort":"high"}`)); err != nil {
			t.Fatal(err)
		}
		msgs := rec.calls("/sessions/s1/message")
		if len(msgs) != 1 || msgs[0].body["text"] != "/effort high" {
			t.Fatalf("want one `/effort high` message, got %+v", msgs)
		}
		if n := len(rec.calls("/sessions/s1/model")); n != 0 {
			t.Errorf("a claude session hit the /model endpoint %d time(s)", n)
		}
	})

	t.Run("codex uses the model endpoint", func(t *testing.T) {
		rec := newRecorder()
		srv := rec.server()
		defer srv.Close()
		reg := newRegistry(newClaudemonClient(srv.URL))
		reg.store = newSessionStore()
		reg.store.set("s2", json.RawMessage(`{"session_id":"s2","provider":"codex"}`))

		if _, err := reg.handle(context.Background(), "claude.setEffort",
			json.RawMessage(`{"sessionId":"s2","effort":"xhigh"}`)); err != nil {
			t.Fatal(err)
		}
		models := rec.calls("/sessions/s2/model")
		if len(models) != 1 || models[0].body["effort"] != "xhigh" {
			t.Fatalf("want one /model post carrying the effort, got %+v", models)
		}
		// Empty fields are OMITTED, not sent as "": a present-but-empty model is
		// a request to switch to a model with no name.
		if _, present := models[0].body["model"]; present {
			t.Errorf("an empty model was sent to the daemon: %+v", models[0].body)
		}
		if n := len(rec.calls("/sessions/s2/message")); n != 0 {
			t.Errorf("a managed session went through the message path %d time(s)", n)
		}
	})
}

// setModel omits what it was not given, for the same reason.
func TestSetModelOmitsFieldsItWasNotGiven(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"gpt-5.5"}`)); err != nil {
		t.Fatal(err)
	}
	hits := rec.calls("/sessions/s1/model")
	if len(hits) != 1 || hits[0].body["model"] != "gpt-5.5" {
		t.Fatalf("want one /model post, got %+v", hits)
	}
	if hits[0].body["model_identity"] != "gpt-5.5" {
		t.Fatalf("legacy-only input was not healed to a canonical identity: %+v", hits[0].body)
	}
	if _, present := hits[0].body["effort"]; present {
		t.Errorf("an empty effort was sent: %+v", hits[0].body)
	}
	// And a call naming neither is refused rather than posting an empty object.
	if _, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1"}`)); err == nil {
		t.Error("claude.setModel accepted a call naming neither a model nor an effort")
	}
}

func TestSetModelRejectsWhitespaceAndControlsBeforeClaudemon(t *testing.T) {
	for name, payload := range map[string]string{
		"whitespace": `{"sessionId":"s1","model":"   "}`,
		"newline":    `{"sessionId":"s1","model":"opus\n/help"}`,
		"paste-end":  `{"sessionId":"s1","model":"opus\u001b[201~/help"}`,
	} {
		t.Run(name, func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			reg := newRegistry(newClaudemonClient(srv.URL))
			res, err := reg.handle(context.Background(), "claude.setModel", json.RawMessage(payload))
			if err != nil {
				t.Fatal(err)
			}
			var out liveControlResult
			_ = json.Unmarshal(res, &out)
			want := "invalid-model-identity"
			if name == "whitespace" {
				want = "empty-model"
			}
			if out.OK || out.Error != want {
				t.Fatalf("result = %+v, want stable %s refusal", out, want)
			}
			if hits := rec.calls("/sessions/s1/model"); len(hits) != 0 {
				t.Fatalf("invalid input mutated the daemon queue: %+v", hits)
			}
		})
	}
}

func TestSetModelCarriesQueuedDispositionWithoutClaimingLiveControl(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`{"ok":true,"queued":true,"disposition":"queued","model":"opus[1m]","requested_selection":{"model":"opus","context_window":1000000}}`))
	}))
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.store = newSessionStore()
	reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude"}`))
	reg.meta = newMetaStore()

	res, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"opus[1m]"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out liveControlResult
	_ = json.Unmarshal(res, &out)
	if !out.OK || !out.Queued || out.Disposition != "queued" || out.RequestedSelection == nil {
		t.Fatalf("queued owner truth was lost: %+v", out)
	}
	if meta, ok := reg.meta.get("s1"); ok && (meta.RequestedModel != "" || meta.LiveEffort != "") {
		t.Fatalf("queued work was recorded as provider execution: %+v", meta)
	}
}

func TestSetModelMapsPrePhase5PTYRefusalToUpgradeRequired(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusConflict)
		_, _ = w.Write([]byte(`{"ok":false,"error":"PTY sessions switch via the /model slash command on the message path"}`))
	}))
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.store = newSessionStore()
	reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude"}`))
	res, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"opus"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out liveControlResult
	_ = json.Unmarshal(res, &out)
	if out.OK || !strings.Contains(out.Error, "upgrade-required") {
		t.Fatalf("pre-Phase-5 skew was not explicit: %+v", out)
	}
}

func TestSetModelDoesNotClaimAnOwnerRefusedPTYEffort(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusConflict)
		_, _ = w.Write([]byte(`{"ok":false,"error":"claude-pty-effort-unsupported: the PTY model command cannot deliver effort"}`))
	}))
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.store = newSessionStore()
	reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude","transport":"pty"}`))
	reg.meta = newMetaStore()
	res, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"opus","effort":"high"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out liveControlResult
	_ = json.Unmarshal(res, &out)
	if out.OK || !strings.Contains(out.Error, "claude-pty-effort-unsupported") || out.Effort != "" {
		t.Fatalf("unapplied effort was claimed: %+v", out)
	}
	if meta, ok := reg.meta.get("s1"); ok && (meta.RequestedModel != "" || meta.LiveEffort != "") {
		t.Fatalf("refused effort mutated brain telemetry: %+v", meta)
	}
}

func TestSetModelTreatsBlankEffortAsAbsent(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.store = newSessionStore()
	reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude","transport":"pty"}`))

	if _, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"opus","effort":"  \t "}`)); err != nil {
		t.Fatal(err)
	}
	hits := rec.calls("/sessions/s1/model")
	if len(hits) != 1 {
		t.Fatalf("setModel calls = %d, want 1", len(hits))
	}
	if _, present := hits[0].body["effort"]; present {
		t.Fatalf("blank effort reached claudemon as substantive input: %+v", hits[0].body)
	}
}

func TestSetModelForwardsCanonicalPairAndMarkerCompanion(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.store = newSessionStore()
	reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude"}`))

	res, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"opus[1m]","modelIdentity":"opus","contextWindow":1000000}`))
	if err != nil {
		t.Fatal(err)
	}
	hits := rec.calls("/sessions/s1/model")
	if len(hits) != 1 || hits[0].body["model"] != "opus[1m]" ||
		hits[0].body["model_identity"] != "opus" || hits[0].body["context_window"] != float64(1_000_000) {
		t.Fatalf("pair/companion drifted on the managed switch wire: %+v", hits)
	}
	var out liveControlResult
	if err := json.Unmarshal(res, &out); err != nil {
		t.Fatal(err)
	}
	if out.RequestedSelection == nil || out.RequestedSelection.Model != "opus" ||
		out.RequestedSelection.ContextWindow == nil || *out.RequestedSelection.ContextWindow != 1_000_000 {
		t.Fatalf("canonical result missing: %+v", out)
	}
}

func TestSetModelForwardsDaemonAcceptedOwnerSelection(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/sessions/s1/model" {
			http.NotFound(w, r)
			return
		}
		_, _ = w.Write([]byte(`{"ok":true,"model":"fable","requested_selection":{"model":"fable","context_window":1000000}}`))
	}))
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.store = newSessionStore()
	reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude"}`))

	res, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"opus[1m]","modelIdentity":"opus","contextWindow":1000000}`))
	if err != nil {
		t.Fatal(err)
	}
	var out liveControlResult
	if err := json.Unmarshal(res, &out); err != nil {
		t.Fatal(err)
	}
	if out.Model != "fable" || out.RequestedSelection == nil ||
		out.RequestedSelection.Model != "fable" || out.RequestedSelection.ContextWindow == nil ||
		*out.RequestedSelection.ContextWindow != 1_000_000 {
		t.Fatalf("brain guessed instead of forwarding owner truth: %+v", out)
	}
}

func TestSetModelRefusesConflictingPairBeforeClaudemon(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.store = newSessionStore()
	reg.store.set("s1", json.RawMessage(`{"session_id":"s1","provider":"claude"}`))

	res, err := reg.handle(context.Background(), "claude.setModel",
		json.RawMessage(`{"sessionId":"s1","model":"opus[1m]","modelIdentity":"sonnet","contextWindow":1000000}`))
	if err != nil {
		t.Fatal(err)
	}
	var out liveControlResult
	_ = json.Unmarshal(res, &out)
	if out.OK || out.Error != "conflicting-model-identity" {
		t.Fatalf("want a stable pair conflict, got %+v", out)
	}
	if hits := rec.calls("/sessions/s1/model"); len(hits) != 0 {
		t.Fatalf("conflicting generations reached claudemon: %+v", hits)
	}
}

// handoffBrief is a pure relay: the caller names a session and never a path.
func TestHandoffBriefRelaysAndNamesNoPath(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.handoffBrief",
		json.RawMessage(`{"sessionId":"s1"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(rec.calls("/sessions/s1/handoff")); n != 1 {
		t.Fatalf("expected one POST to the daemon's handoff endpoint, got %d", n)
	}
	if _, err := reg.handle(context.Background(), "claude.handoffBrief", json.RawMessage(`{}`)); err == nil {
		t.Error("claude.handoffBrief accepted a call with no session id")
	}
}
