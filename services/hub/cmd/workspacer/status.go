package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/busclient"
)

// componentStatus is one probed component in `workspacer status`.
type componentStatus struct {
	OK     bool   `json:"ok"`
	Detail string `json:"detail"`
}

// statusReport is the whole probe result; --json emits it verbatim.
type statusReport struct {
	Claudemon componentStatus `json:"claudemon"`
	Hub       componentStatus `json:"hub"`
	Brain     componentStatus `json:"brain"`
}

// runStatus probes the default (or flagged) ports and reports what's running.
// Exit code 0 = claudemon + hub both healthy (the brain line is informative:
// a hub without a brain is still a working bus).
func runStatus(args []string) int {
	fs := flag.NewFlagSet("workspacer status", flag.ExitOnError)
	host := fs.String("host", "127.0.0.1", "host to probe")
	hubPort := fs.Int("hub-port", 7895, "hub port")
	apiPort := fs.Int("claudemon-api-port", 7891, "claudemon API port")
	token := fs.String("token", os.Getenv("HUB_TOKEN"),
		"bus auth token (default: $HUB_TOKEN, else the persisted <config>/workspacer/remote-token if present)")
	jsonOut := fs.Bool("json", false, "print the report as JSON")
	_ = fs.Parse(args)

	// Reuse the persisted pairing token so a bare `workspacer status` can probe
	// a server started by `workspacer serve` (or the desktop) without flags.
	if *token == "" {
		if b, err := os.ReadFile(configDir() + "/remote-token"); err == nil {
			*token = strings.TrimSpace(string(b))
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	hubBase := "http://" + net.JoinHostPort(*host, fmt.Sprintf("%d", *hubPort))
	claudemonBase := "http://" + net.JoinHostPort(*host, fmt.Sprintf("%d", *apiPort))

	report := statusReport{
		Claudemon: probeClaudemon(ctx, claudemonBase),
		Hub:       probeHub(ctx, hubBase, *token),
	}
	if report.Hub.OK {
		busURL := "ws://" + net.JoinHostPort(*host, fmt.Sprintf("%d", *hubPort)) + "/bus"
		report.Brain = probeBrain(ctx, busURL, *token)
	} else {
		report.Brain = componentStatus{OK: false, Detail: "not checked (hub is down)"}
	}

	if *jsonOut {
		_ = json.NewEncoder(os.Stdout).Encode(report)
	} else {
		fmt.Print(renderStatus(report, claudemonBase, hubBase))
	}
	if report.Claudemon.OK && report.Hub.OK {
		return 0
	}
	return 1
}

// renderStatus formats the human-readable report. Pure, for tests.
func renderStatus(r statusReport, claudemonBase, hubBase string) string {
	var sb strings.Builder
	row := func(name string, s componentStatus, base string) {
		state := "down"
		if s.OK {
			state = "up"
		}
		fmt.Fprintf(&sb, "  %-10s %-5s %s", name, state, s.Detail)
		if base != "" {
			fmt.Fprintf(&sb, " — %s", base)
		}
		sb.WriteString("\n")
	}
	row("claudemon", r.Claudemon, claudemonBase)
	row("hub", r.Hub, hubBase)
	row("brain", r.Brain, "")
	return sb.String()
}

// probeClaudemon checks /health, then counts /sessions (the useful headline:
// "is it up, and is anything running on it").
func probeClaudemon(ctx context.Context, base string) componentStatus {
	if err := httpOK(ctx, base+"/health", ""); err != nil {
		return componentStatus{OK: false, Detail: "not running (" + probeErr(err) + ")"}
	}
	body, err := httpGet(ctx, base+"/sessions", "")
	if err != nil {
		return componentStatus{OK: true, Detail: "healthy (sessions unreadable: " + probeErr(err) + ")"}
	}
	var sessions []json.RawMessage
	if err := json.Unmarshal(body, &sessions); err != nil {
		return componentStatus{OK: true, Detail: "healthy"}
	}
	return componentStatus{OK: true, Detail: fmt.Sprintf("healthy, %d session(s)", len(sessions))}
}

// probeHub checks /health with the token: an authorized probe gets the
// registered-method count back, which is a cheap topology headline.
func probeHub(ctx context.Context, base, token string) componentStatus {
	body, err := httpGet(ctx, base+"/health", token)
	if err != nil {
		return componentStatus{OK: false, Detail: "not running (" + probeErr(err) + ")"}
	}
	var h struct {
		Status  string `json:"status"`
		Methods *int   `json:"methods"`
	}
	if err := json.Unmarshal(body, &h); err != nil || h.Status != "ok" {
		return componentStatus{OK: false, Detail: "unexpected /health answer"}
	}
	if h.Methods == nil {
		// The hub hides counts from unauthorized probes — still alive, though.
		return componentStatus{OK: true, Detail: "healthy (token not accepted — method count hidden)"}
	}
	return componentStatus{OK: true, Detail: fmt.Sprintf("healthy, %d capability method(s)", *h.Methods)}
}

// probeBrain asks the bus for a brain-provided capability. app.getCwd is the
// cheapest one with no side effects; the hub answering "no provider" is the
// definitive "no brain registered" signal (method counts can't distinguish
// the hub's own local methods from a live brain).
func probeBrain(ctx context.Context, busURL, token string) componentStatus {
	cctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	cli := busclient.New(busURL, token)
	go cli.Run(cctx)
	_, err := cli.Call(cctx, "app.getCwd", struct{}{})
	switch {
	case err == nil:
		return componentStatus{OK: true, Detail: "registered (app.getCwd answered)"}
	case strings.Contains(err.Error(), "no provider"):
		return componentStatus{OK: false, Detail: "not registered (hub is up but no provider answered — is the brain running?)"}
	case errors.Is(err, busclient.ErrNotConnected):
		return componentStatus{OK: false, Detail: "bus unreachable (wrong token?)"}
	default:
		return componentStatus{OK: false, Detail: "probe failed (" + probeErr(err) + ")"}
	}
}

// httpGet fetches url (Bearer token when non-empty) and returns the body.
func httpGet(ctx context.Context, url, token string) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	client := &http.Client{Timeout: 3 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("HTTP %d", resp.StatusCode)
	}
	return io.ReadAll(io.LimitReader(resp.Body, 1<<20))
}

// httpOK is httpGet without caring about the body.
func httpOK(ctx context.Context, url, token string) error {
	_, err := httpGet(ctx, url, token)
	return err
}

// probeErr compresses Go's verbose net errors into a short reason
// ("connection refused" instead of the full dial chain).
func probeErr(err error) string {
	msg := err.Error()
	if i := strings.LastIndex(msg, ": "); i >= 0 {
		return msg[i+2:]
	}
	return msg
}
