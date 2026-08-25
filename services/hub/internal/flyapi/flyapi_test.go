package flyapi

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// EVERY TEST IN THIS FILE TALKS TO A STUB. Nothing here has, or wants,
// credentials for the real Fly, and nothing here may create, start or stop
// anything that costs money. BaseURL is pointed at an httptest server and the
// token is a literal that exists to be searched for in output.
const fakeToken = "fly_test_TOKEN_MUST_NEVER_ESCAPE"

func stub(t *testing.T, h http.HandlerFunc) *HTTP {
	t.Helper()
	srv := httptest.NewServer(h)
	t.Cleanup(srv.Close)
	c := New(fakeToken)
	c.BaseURL = srv.URL
	// The rate gate is real behaviour, but a test suite should not pay a real
	// second for it; record the waits instead of sleeping them.
	c.sleep = func(context.Context, time.Duration) {}
	return c
}

func TestStartPostsToTheMachineStartPathWithABearerToken(t *testing.T) {
	var gotPath, gotMethod, gotAuth, gotQuery string
	c := stub(t, func(w http.ResponseWriter, r *http.Request) {
		gotPath, gotMethod, gotAuth, gotQuery = r.URL.Path, r.Method, r.Header.Get("Authorization"), r.URL.RawQuery
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"ok":true}`))
	})
	if err := c.Start(context.Background(), "wks-node", "17811944b12345"); err != nil {
		t.Fatalf("Start: %v", err)
	}
	if gotMethod != http.MethodPost {
		t.Errorf("method = %q, want POST", gotMethod)
	}
	if want := "/v1/apps/wks-node/machines/17811944b12345/start"; gotPath != want {
		t.Errorf("path = %q, want %q", gotPath, want)
	}
	if want := "Bearer " + fakeToken; gotAuth != want {
		t.Errorf("Authorization = %q, want %q", gotAuth, want)
	}
	// THE TOKEN MUST TRAVEL IN A HEADER, NEVER IN A URL: a URL lands in proxy
	// logs, in Fly's own request logs, and in any error string that quotes it.
	if strings.Contains(gotQuery, fakeToken) || strings.Contains(gotPath, fakeToken) {
		t.Errorf("the token appeared in the request URL (%q %q)", gotPath, gotQuery)
	}
}

func TestStateReadsTheMachineStateString(t *testing.T) {
	c := stub(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			t.Errorf("State used %s, want GET", r.Method)
		}
		_, _ = w.Write([]byte(`{"id":"17811944b12345","state":"stopped"}`))
	})
	got, err := c.State(context.Background(), "wks-node", "17811944b12345")
	if err != nil {
		t.Fatalf("State: %v", err)
	}
	if got != StateStopped {
		t.Errorf("state = %q, want %q", got, StateStopped)
	}
}

// An unrecognised state string is returned as-is rather than coerced into one
// of ours: a registry that renders "asleep and fine" for a state Fly invented
// last month is worse than one that says a word the user can search for.
func TestStatePassesThroughAnUnknownState(t *testing.T) {
	c := stub(t, func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`{"state":"some-new-fly-state"}`))
	})
	got, _ := c.State(context.Background(), "a", "b")
	if got != "some-new-fly-state" {
		t.Errorf("state = %q, want it passed through verbatim", got)
	}
}

func TestWaitForStateUsesFlysOwnWaitEndpoint(t *testing.T) {
	var gotPath string
	var gotState, gotTimeout string
	c := stub(t, func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		gotState = r.URL.Query().Get("state")
		gotTimeout = r.URL.Query().Get("timeout")
		w.WriteHeader(http.StatusOK)
	})
	if err := c.WaitForState(context.Background(), "wks-node", "m1", StateStarted, 45*time.Second); err != nil {
		t.Fatalf("WaitForState: %v", err)
	}
	if want := "/v1/apps/wks-node/machines/m1/wait"; gotPath != want {
		t.Errorf("path = %q, want %q", gotPath, want)
	}
	if gotState != StateStarted {
		t.Errorf("state = %q, want %q", gotState, StateStarted)
	}
	if gotTimeout != "45" {
		t.Errorf("timeout = %q, want 45", gotTimeout)
	}
}

// Fly caps one /wait at 60s. Asking for more must not be discovered in
// production.
func TestWaitForStateClampsTheTimeoutToFlysCeiling(t *testing.T) {
	var gotTimeout string
	c := stub(t, func(w http.ResponseWriter, r *http.Request) {
		gotTimeout = r.URL.Query().Get("timeout")
		w.WriteHeader(http.StatusOK)
	})
	_ = c.WaitForState(context.Background(), "a", "b", StateStarted, 10*time.Minute)
	if gotTimeout != "60" {
		t.Errorf("timeout = %q, want it clamped to 60", gotTimeout)
	}
}

func TestNotFoundIsDistinguishableFromATransientFailure(t *testing.T) {
	c := stub(t, func(w http.ResponseWriter, _ *http.Request) {
		http.Error(w, `{"error":"machine not found"}`, http.StatusNotFound)
	})
	err := c.Start(context.Background(), "wks-node", "nope")
	var apiErr *APIError
	if !errors.As(err, &apiErr) {
		t.Fatalf("Start error = %v, want an *APIError", err)
	}
	if !apiErr.NotFound() {
		t.Errorf("NotFound() = false for a 404")
	}
}

func TestRateLimitIsItsOwnErrorWithRetryAfter(t *testing.T) {
	c := stub(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Retry-After", "3")
		w.WriteHeader(http.StatusTooManyRequests)
	})
	err := c.Start(context.Background(), "wks-node", "m1")
	var rl *RateLimitError
	if !errors.As(err, &rl) {
		t.Fatalf("Start error = %v, want a *RateLimitError", err)
	}
	if rl.RetryAfter != 3*time.Second {
		t.Errorf("RetryAfter = %v, want 3s", rl.RetryAfter)
	}
}

// The rate gate holds a second action for the same machine back rather than
// letting it earn a 429.
func TestActionsForOneMachineAreSpacedOut(t *testing.T) {
	var waits []time.Duration
	c := stub(t, func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK) })
	c.sleep = func(_ context.Context, d time.Duration) { waits = append(waits, d) }
	ctx := context.Background()
	_ = c.Start(ctx, "app", "m1")
	_ = c.Start(ctx, "app", "m1")
	if len(waits) != 1 || waits[0] <= 0 {
		t.Fatalf("second Start for the same machine waited %v, want one positive wait", waits)
	}
	// A different machine is a different limit and must not be held back.
	waits = nil
	_ = c.Start(ctx, "app", "m2")
	if len(waits) != 0 {
		t.Errorf("a different machine was rate-gated behind m1: %v", waits)
	}
}

// THE TOKEN-LEAK TEST.
//
// An error body is attacker-adjacent data: it is composed by whatever answered
// the request, which on the way to Fly may be a corporate proxy, a captive
// portal, or a debug gateway that helpfully echoes the request it received —
// headers included. Anything this client puts in an error can end up in a hub
// log, and the hub log is read by people who are not supposed to hold this
// credential.
//
// So no error out of this package may contain the token, whatever the far end
// sent back.
func TestNoErrorEverCarriesTheToken(t *testing.T) {
	echo := func(w http.ResponseWriter, r *http.Request) {
		// The shape a debug proxy actually produces.
		http.Error(w, `{"error":"upstream refused","request":{"headers":{"authorization":"`+
			r.Header.Get("Authorization")+`"}}}`, http.StatusBadGateway)
	}
	ctx := context.Background()
	c := stub(t, echo)

	var errs []error
	errs = append(errs, c.Start(ctx, "app", "m1"))
	_, stateErr := c.State(ctx, "app", "m1")
	errs = append(errs, stateErr)
	errs = append(errs, c.WaitForState(ctx, "app", "m1", StateStarted, time.Second))

	for i, err := range errs {
		if err == nil {
			t.Fatalf("call %d: expected an error from a 502", i)
		}
		if strings.Contains(err.Error(), fakeToken) {
			t.Errorf("call %d LEAKED THE FLY TOKEN into an error string: %s", i, err.Error())
		}
		if !strings.Contains(err.Error(), "502") {
			t.Errorf("call %d error lost the status code: %s", i, err.Error())
		}
	}
}

// The same rule for a transport-level failure, where the error text is
// composed by net/http out of the request URL.
func TestATransportFailureDoesNotCarryTheToken(t *testing.T) {
	c := New(fakeToken)
	c.BaseURL = "http://127.0.0.1:1" // nothing listens here
	c.HTTPClient = &http.Client{Timeout: 500 * time.Millisecond}
	err := c.Start(context.Background(), "app", "m1")
	if err == nil {
		t.Fatal("expected a connection failure")
	}
	if strings.Contains(err.Error(), fakeToken) {
		t.Errorf("a transport error carried the token: %s", err.Error())
	}
}

// The client cannot express an IRREVERSIBLE verb, and that is the line this
// package draws — not "read only", which stopped being true the day the sleep
// path landed.
//
// Start and Stop are each other's undo. Destroy and Delete have no undo at
// all, so a bug in the layer above cannot be talked into one: there is no
// method to reach.
func TestClientInterfaceOffersNoIrreversibleVerb(t *testing.T) {
	var c Client = New("x")
	if _, ok := any(c).(interface {
		Destroy(context.Context, string, string) error
	}); ok {
		t.Error("flyapi grew a Destroy verb; nothing in this hub may delete a machine")
	}
	if _, ok := any(c).(interface {
		Delete(context.Context, string, string) error
	}); ok {
		t.Error("flyapi grew a Delete verb; nothing in this hub may delete a machine")
	}
	if _, ok := any(c).(interface {
		Create(context.Context, string, string) error
	}); ok {
		t.Error("flyapi grew a Create verb; nothing in this hub may create a machine (it would create a bill nobody asked for)")
	}
}

// THE MUTATION GUARD FOR THE ONE API DETAIL THAT IS EXPENSIVE TO GET WRONG.
//
// fly.toml's kill_signal / kill_timeout govern a PLATFORM stop. A stop issued
// through the Machines API never reads that file and takes its own `signal`
// and `timeout`, so a Stop that lets a caller omit them is a Stop that
// silently substitutes the API's defaults for the deployment's drain window —
// and a node SIGKILLed mid-flush writes no exit record, which is precisely the
// signal the reconciler needs to tell a deliberate sleep from a crash.
//
// This test fails if Stop is ever given a default to fall through to.
func TestEveryStopTakesAnExplicitSignalAndTimeout(t *testing.T) {
	var calls int
	c := stub(t, func(w http.ResponseWriter, r *http.Request) {
		calls++
		w.WriteHeader(http.StatusOK)
	})
	if err := c.Stop(context.Background(), "app", "m1", "", 30*time.Second); err == nil {
		t.Error("Stop accepted an EMPTY SIGNAL — fly.toml's kill_signal does not govern an API stop, so there is nothing for it to fall back to")
	}
	if err := c.Stop(context.Background(), "app", "m1", "SIGTERM", 0); err == nil {
		t.Error("Stop accepted a ZERO TIMEOUT — fly.toml's kill_timeout does not govern an API stop, so a zero here is the API's default drain window and not the deployment's")
	}
	if err := c.Stop(context.Background(), "app", "m1", "SIGTERM", -1); err == nil {
		t.Error("Stop accepted a NEGATIVE timeout")
	}
	if calls != 0 {
		t.Errorf("a refused Stop still issued %d request(s) — it must refuse BEFORE it spends an action against the rate limit", calls)
	}
}

func TestStopPostsSignalAndTimeoutToTheMachineStopPath(t *testing.T) {
	var gotPath, gotMethod, gotAuth, gotQuery, gotBody, gotType string
	c := stub(t, func(w http.ResponseWriter, r *http.Request) {
		gotPath, gotMethod, gotAuth, gotQuery = r.URL.Path, r.Method, r.Header.Get("Authorization"), r.URL.RawQuery
		gotType = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		gotBody = string(b)
		w.WriteHeader(http.StatusOK)
	})
	if err := c.Stop(context.Background(), "wks-node", "17811944b12345", "SIGTERM", 45*time.Second); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if gotMethod != http.MethodPost {
		t.Errorf("method = %q, want POST", gotMethod)
	}
	if want := "/v1/apps/wks-node/machines/17811944b12345/stop"; gotPath != want {
		t.Errorf("path = %q, want %q", gotPath, want)
	}
	if want := "Bearer " + fakeToken; gotAuth != want {
		t.Errorf("Authorization = %q, want %q", gotAuth, want)
	}
	if gotType != "application/json" {
		t.Errorf("Content-Type = %q, want application/json", gotType)
	}
	// The signal and the drain window must actually be ON THE WIRE. A Stop
	// that validates them and then does not send them is the same bug with a
	// green unit test in front of it.
	if !strings.Contains(gotBody, `"signal":"SIGTERM"`) {
		t.Errorf("the stop body did not carry the signal: %s", gotBody)
	}
	if !strings.Contains(gotBody, `"timeout":"45s"`) {
		t.Errorf("the stop body did not carry the drain timeout: %s", gotBody)
	}
	// Same rule as Start: the credential is a header, never a URL.
	if strings.Contains(gotQuery, fakeToken) || strings.Contains(gotPath, fakeToken) {
		t.Errorf("the token appeared in the request URL (%q %q)", gotPath, gotQuery)
	}
	if strings.Contains(gotBody, fakeToken) {
		t.Errorf("the token appeared in the request BODY: %s", gotBody)
	}
}

// A failing stop is an error path like any other, and it goes through the same
// scrub. Written against the stub that echoes the request headers back — the
// shape a debug proxy or captive portal actually produces.
func TestAFailingStopDoesNotCarryTheToken(t *testing.T) {
	c := stub(t, func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusBadGateway)
		_, _ = w.Write([]byte(`{"error":"upstream refused","request":{"headers":{"authorization":"` +
			r.Header.Get("Authorization") + `"}}}`))
	})
	err := c.Stop(context.Background(), "app", "m1", "SIGTERM", 30*time.Second)
	if err == nil {
		t.Fatal("expected an error from a 502")
	}
	if strings.Contains(err.Error(), fakeToken) {
		t.Errorf("a failed Stop LEAKED THE FLY TOKEN: %s", err.Error())
	}
	var apiErr *APIError
	if errors.As(err, &apiErr) && strings.Contains(apiErr.Body, fakeToken) {
		t.Errorf("APIError.Body carried the token: %s", apiErr.Body)
	}
}

// A stop is an ACTION, so it is gated at one per second per machine like every
// other action — not at the 5/s read rate. Three stops in a row must wait.
func TestStopIsRateGatedAsAnAction(t *testing.T) {
	var waits []time.Duration
	c := stub(t, func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK) })
	c.sleep = func(_ context.Context, d time.Duration) { waits = append(waits, d) }
	for i := 0; i < 3; i++ {
		if err := c.Stop(context.Background(), "app", "m1", "SIGTERM", time.Second); err != nil {
			t.Fatalf("stop %d: %v", i, err)
		}
	}
	if len(waits) < 2 {
		t.Fatalf("three stops in a row waited %d time(s); Fly allows one action per second per machine and a 429 on this path reads as \"the button does nothing\"", len(waits))
	}
	// The gate schedules cumulatively (each call reserves its own slot), so
	// the waits grow — what matters is that every one of them is at the ACTION
	// interval's granularity and not the 200ms read one.
	for i, w := range waits {
		if w <= readMinInterval*2 {
			t.Errorf("stop %d waited %s, which is the READ interval's order (%s) and not the action interval's (%s) — a stop has been gated as a read", i, w, readMinInterval, actionMinInterval)
		}
	}
}

// THE TWO SPELLINGS. fly.toml says `on-failure`; the Machines API says
// `on-fail`. They are the same setting and they are not the same string, and
// a machine that will not stay up is an expensive place to discover that.
//
// This holds the constants against the deployment file itself, so the pair
// cannot drift into agreement (which would be wrong) or out of the file's
// prose (which would be worse — the file is where the next person looks).
func TestTheTwoRestartPolicySpellingsAreBothRecorded(t *testing.T) {
	if RestartPolicyOnFailureTOML == RestartPolicyOnFailAPI {
		t.Fatal("the two restart-policy spellings have been collapsed into one; fly.toml says on-failure and the Machines API says on-fail, and they are not interchangeable")
	}
	// ../../../../deploy/fly/node/fly.toml from services/hub/internal/flyapi.
	raw, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "deploy", "fly", "node", "fly.toml"))
	if err != nil {
		t.Skipf("node fly.toml not readable from here: %v", err)
	}
	toml := string(raw)
	if !strings.Contains(toml, `policy  = "`+RestartPolicyOnFailureTOML+`"`) &&
		!strings.Contains(toml, `policy = "`+RestartPolicyOnFailureTOML+`"`) {
		t.Errorf("deploy/fly/node/fly.toml no longer sets policy = %q — either the deployment changed its restart policy (in which case internal/nodes' crash-vs-sleep reasoning needs re-reading) or this constant is now a lie", RestartPolicyOnFailureTOML)
	}
	if !strings.Contains(toml, RestartPolicyOnFailAPI) {
		t.Errorf("deploy/fly/node/fly.toml no longer mentions the API spelling %q; the difference between the two is exactly the thing that file's comment exists to record", RestartPolicyOnFailAPI)
	}
}
