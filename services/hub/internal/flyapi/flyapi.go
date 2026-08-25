// Package flyapi is a small client for the Fly Machines API — the control
// plane that can start a machine that is not running.
//
// It exists because a stopped Fly machine has no tailnet presence at all:
// nothing is executing on it, so there is no tailscaled, no claudemon and no
// brain to reach. The data plane cannot wake the thing it runs on. The wake
// therefore has to travel out-of-band, over the public internet, to Fly's own
// API — and that is the entire reason this package is the first code in this
// repository that talks to a cloud provider.
//
// It is deliberately tiny and deliberately read/start only. There is no stop,
// no create, no destroy and no delete here: the hub's job in v1 is to WAKE a
// node, and a client that cannot express "destroy this machine" cannot be
// talked into it by a bug upstream of it.
package flyapi

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"
)

// DefaultBaseURL is Fly's public Machines API endpoint. From INSIDE a Fly
// org's private network the same API is at http://_api.internal:4280, which is
// why this is configurable rather than a constant in the request builder.
const DefaultBaseURL = "https://api.machines.dev"

// Machine states, as Fly reports them. Only the ones this package reasons
// about are named; State() returns whatever string the API sent, so an
// unrecognised state is surfaced rather than silently coerced.
const (
	StateStarted   = "started"
	StateStopped   = "stopped"
	StateSuspended = "suspended"
	StateStarting  = "starting"
	StateReplacing = "replacing"
	StateDestroyed = "destroyed"
)

// waitMaxTimeout is the ceiling Fly puts on one /wait call. Asking for more is
// not an error we want to discover in production, so the client clamps.
const waitMaxTimeout = 60 * time.Second

// Rate limits, from Fly's Machines API documentation: one request per second
// per ACTION per machine (short bursts to 3), and five per second for reading
// a machine (bursts to 10). Exceeding them earns a 429, and a 429 on the wake
// path reads to a user as "the button does nothing".
//
// The gate below is a minimum interval per (action, machine) rather than a
// token bucket: it gives up the burst allowance in exchange for being
// obviously correct, and nothing here needs a burst. The supervisor serialises
// wakes per node anyway; this is the floor under that, not the plan.
const (
	actionMinInterval = time.Second
	readMinInterval   = 200 * time.Millisecond
)

// Client is the slice of the Machines API the hub uses. An interface so the
// node supervisor can be tested against a fake, and so nothing in a test can
// reach the real Fly.
type Client interface {
	// Start asks Fly to start a machine. It returns as soon as Fly has
	// accepted the request; the machine is not running yet.
	Start(ctx context.Context, app, machineID string) error
	// State reports the machine's current state string (see the State*
	// constants).
	State(ctx context.Context, app, machineID string) (string, error)
	// WaitForState blocks until the machine reaches want, or the timeout
	// expires. This is Fly's own /wait endpoint, which is why it is here
	// rather than being a poll loop in the caller: polling GET at more than
	// 5/s earns a 429, and one blocking call earns nothing.
	WaitForState(ctx context.Context, app, machineID, want string, timeout time.Duration) error
}

// HTTP is the real client.
type HTTP struct {
	// BaseURL defaults to DefaultBaseURL when empty.
	BaseURL string
	// Token is the Fly API token. It is sent as an Authorization header and is
	// never placed in a URL, never logged, and scrubbed out of every error
	// this package returns — see scrub.
	Token string
	// HTTPClient defaults to a client with a sane timeout.
	HTTPClient *http.Client

	mu       sync.Mutex
	lastCall map[string]time.Time
	// sleep is time.Sleep in production; tests replace it so the rate gate
	// does not make the suite slow.
	sleep func(context.Context, time.Duration)
}

// New builds a client for a token.
func New(token string) *HTTP { return &HTTP{Token: token} }

func (c *HTTP) base() string {
	if c.BaseURL != "" {
		return strings.TrimRight(c.BaseURL, "/")
	}
	return DefaultBaseURL
}

func (c *HTTP) client() *http.Client {
	if c.HTTPClient != nil {
		return c.HTTPClient
	}
	return &http.Client{Timeout: 30 * time.Second}
}

// gate enforces the per-(action, machine) minimum interval.
func (c *HTTP) gate(ctx context.Context, key string, min time.Duration) {
	c.mu.Lock()
	if c.lastCall == nil {
		c.lastCall = map[string]time.Time{}
	}
	now := time.Now()
	wait := time.Duration(0)
	if last, ok := c.lastCall[key]; ok {
		if d := min - now.Sub(last); d > 0 {
			wait = d
		}
	}
	c.lastCall[key] = now.Add(wait)
	c.mu.Unlock()
	if wait <= 0 {
		return
	}
	if c.sleep != nil {
		c.sleep(ctx, wait)
		return
	}
	t := time.NewTimer(wait)
	defer t.Stop()
	select {
	case <-ctx.Done():
	case <-t.C:
	}
}

// machinePath builds /v1/apps/{app}/machines/{id}{suffix}. Both identifiers
// are path-escaped: they come from a config file a person edits, and a stray
// slash must not be able to redirect a POST at a different resource.
func (c *HTTP) machinePath(app, machineID, suffix string) string {
	return c.base() + "/v1/apps/" + url.PathEscape(app) + "/machines/" + url.PathEscape(machineID) + suffix
}

func (c *HTTP) do(ctx context.Context, method, urlStr string, wantJSON any) error {
	req, err := http.NewRequestWithContext(ctx, method, urlStr, nil)
	if err != nil {
		return c.scrub(err)
	}
	if c.Token != "" {
		req.Header.Set("Authorization", "Bearer "+c.Token)
	}
	req.Header.Set("Accept", "application/json")
	resp, err := c.client().Do(req)
	if err != nil {
		return c.scrub(err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 64<<10))
	if resp.StatusCode == http.StatusTooManyRequests {
		return &RateLimitError{RetryAfter: parseRetryAfter(resp.Header.Get("Retry-After")), Status: resp.StatusCode}
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		// Scrubbed at CONSTRUCTION, not at Error(): the field is exported, and
		// a caller that reads .Body directly must get the redacted text too.
		return &APIError{Status: resp.StatusCode, Body: scrubText(summarise(string(body)), c.Token)}
	}
	if wantJSON != nil && len(body) > 0 {
		if err := json.Unmarshal(body, wantJSON); err != nil {
			return c.scrub(fmt.Errorf("fly: unreadable answer (%d): %w", resp.StatusCode, err))
		}
	}
	return nil
}

// Start implements Client.
func (c *HTTP) Start(ctx context.Context, app, machineID string) error {
	if err := requireIDs(app, machineID); err != nil {
		return err
	}
	c.gate(ctx, "start:"+app+"/"+machineID, actionMinInterval)
	return c.do(ctx, http.MethodPost, c.machinePath(app, machineID, "/start"), nil)
}

// State implements Client.
func (c *HTTP) State(ctx context.Context, app, machineID string) (string, error) {
	if err := requireIDs(app, machineID); err != nil {
		return "", err
	}
	c.gate(ctx, "get:"+app+"/"+machineID, readMinInterval)
	var m struct {
		ID    string `json:"id"`
		State string `json:"state"`
	}
	if err := c.do(ctx, http.MethodGet, c.machinePath(app, machineID, ""), &m); err != nil {
		return "", err
	}
	return m.State, nil
}

// WaitForState implements Client.
func (c *HTTP) WaitForState(ctx context.Context, app, machineID, want string, timeout time.Duration) error {
	if err := requireIDs(app, machineID); err != nil {
		return err
	}
	if timeout <= 0 || timeout > waitMaxTimeout {
		timeout = waitMaxTimeout
	}
	q := url.Values{}
	q.Set("state", want)
	q.Set("timeout", strconv.Itoa(int(timeout/time.Second)))
	c.gate(ctx, "wait:"+app+"/"+machineID, readMinInterval)
	return c.do(ctx, http.MethodGet, c.machinePath(app, machineID, "/wait")+"?"+q.Encode(), nil)
}

func requireIDs(app, machineID string) error {
	if strings.TrimSpace(app) == "" || strings.TrimSpace(machineID) == "" {
		return fmt.Errorf("fly: both an app name and a machine id are required")
	}
	return nil
}

// APIError is a non-2xx answer from Fly.
type APIError struct {
	Status int
	Body   string
}

func (e *APIError) Error() string {
	if e.Body == "" {
		return fmt.Sprintf("fly: HTTP %d", e.Status)
	}
	return fmt.Sprintf("fly: HTTP %d: %s", e.Status, e.Body)
}

// NotFound reports whether Fly says the app or machine does not exist — a
// configuration mistake rather than a transient failure, and the one the node
// registry should surface rather than retry.
func (e *APIError) NotFound() bool { return e.Status == http.StatusNotFound }

// RateLimitError is Fly's 429. Separated from APIError because the caller's
// correct response is different: wait, do not re-ask.
type RateLimitError struct {
	Status     int
	RetryAfter time.Duration
}

func (e *RateLimitError) Error() string {
	if e.RetryAfter > 0 {
		return fmt.Sprintf("fly: rate limited (HTTP 429), retry after %s", e.RetryAfter)
	}
	return "fly: rate limited (HTTP 429)"
}

func parseRetryAfter(v string) time.Duration {
	n, err := strconv.Atoi(strings.TrimSpace(v))
	if err != nil || n < 0 {
		return 0
	}
	return time.Duration(n) * time.Second
}

// summarise trims a response body to something that fits in an error string.
func summarise(s string) string {
	s = strings.TrimSpace(strings.ReplaceAll(s, "\n", " "))
	const max = 300
	if len(s) > max {
		return s[:max] + "…"
	}
	return s
}

// Redacted is what a Fly token is replaced by anywhere it would otherwise be
// rendered into text.
const Redacted = "[fly token redacted]"

// scrub removes the token from an error's text.
//
// EVERY error this package returns goes through here, and that is a rule
// rather than a nicety. An error string composed from a response BODY is
// composed by whatever answered the request — which on the way to Fly can be a
// corporate proxy, a captive portal or a debug gateway that echoes the request
// headers back at you. The observed shape is
//
//	{"error":"upstream refused","request":{"headers":{"authorization":"Bearer fly_…"}}}
//
// and without this, that string went straight into the hub's log, which is
// read by people who do not hold this credential and on a machine where it is
// otherwise only ever in one 0600 file. A transport error is the same problem
// from the other side: net/http composes it out of the request URL, which is
// why the token is a header here and never a query parameter.
//
// Redacting a value we hold is the right direction for THIS: unlike a
// projection over a growing struct, the set of secrets is exactly one string
// and we know it. The projection argument (name what goes in, not what comes
// out) applies to the node registry's client-facing views, and that is where
// it is used — see internal/nodes.NodeView.
func (c *HTTP) scrub(err error) error {
	if err == nil || c.Token == "" {
		return err
	}
	msg := err.Error()
	clean := scrubText(msg, c.Token)
	if clean == msg {
		return err
	}
	return errors.New(clean)
}

// scrubText replaces every occurrence of token in s.
func scrubText(s, token string) string {
	if token == "" {
		return s
	}
	return strings.ReplaceAll(s, token, Redacted)
}
