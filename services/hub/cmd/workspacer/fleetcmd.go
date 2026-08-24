package main

// `workspacer fleet quiescence` — read the hub's fleet-quiescence signal from
// a terminal or a script.
//
// The signal itself is a bus method, and a bus method is not reachable from a
// shell. This is the seam that makes it reachable: the exit code answers the
// question, so a job's shell action can be one line.
//
//	workspacer fleet quiescence --quiet && /opt/wks/power-down.sh
//
// It reports and nothing more. It has no idea what the caller does with the
// answer, which is deliberate: what "the fleet is at rest" is worth, and what
// should happen when it is true, belongs to whoever runs this.

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"net"
	"os"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/busclient"
)

const fleetUsage = `workspacer fleet — read the fleet's state

Usage:
  workspacer fleet quiescence [--json] [--quiet]

quiescence answers whether this machine's fleet is genuinely at rest: every
session ready for input, no background task running, nothing waiting on a
human, no live terminal, no bus client in use, no job about to fire, every
federated peer quiet — and all of it held continuously for a dwell of several
minutes. When the answer is no it lists exactly what is holding the machine up.

Exit codes: 0 = at rest, 1 = not at rest, 2 = could not ask (hub unreachable,
wrong token, bad arguments). A caller that acts on success must distinguish 1
from 2: "the fleet is busy" and "I could not find out" are different answers,
and only the first is knowledge.

  --json   print the raw answer ({quiescent, since, blockers[]})
  --quiet  print nothing; use the exit code only

Flags: --host (127.0.0.1), --hub-port (7895), --token ($HUB_TOKEN, else the
persisted <config>/workspacer/remote-token).
`

func runFleet(args []string) int {
	if len(args) < 1 {
		fmt.Fprint(os.Stderr, fleetUsage)
		return 2
	}
	switch args[0] {
	case "quiescence":
		return runFleetQuiescence(args[1:])
	case "help", "-h", "--help":
		fmt.Print(fleetUsage)
		return 0
	default:
		fmt.Fprintf(os.Stderr, "workspacer fleet: unknown subcommand %q\n\n%s", args[0], fleetUsage)
		return 2
	}
}

// quiescenceAnswer mirrors what the bus method returns. Kept as its own struct
// rather than importing the hub's, so the CLI stays a thin client of the wire
// shape.
type quiescenceAnswer struct {
	Quiescent bool   `json:"quiescent"`
	Since     *int64 `json:"since"`
	Blockers  []struct {
		Kind   string `json:"kind"`
		ID     string `json:"id"`
		Detail string `json:"detail"`
	} `json:"blockers"`
	DwellSeconds int64 `json:"dwellSeconds"`
	CalmSeconds  int64 `json:"calmSeconds"`
}

func runFleetQuiescence(args []string) int {
	fs := flag.NewFlagSet("workspacer fleet quiescence", flag.ExitOnError)
	host := fs.String("host", "127.0.0.1", "hub host")
	hubPort := fs.Int("hub-port", 7895, "hub port")
	token := fs.String("token", os.Getenv("HUB_TOKEN"),
		"bus auth token (default: $HUB_TOKEN, else the persisted <config>/workspacer/remote-token)")
	jsonOut := fs.Bool("json", false, "print the raw answer as JSON")
	quiet := fs.Bool("quiet", false, "print nothing; use the exit code")
	flagArgs, _ := splitPositionals(args, map[string]bool{"host": true, "hub-port": true, "token": true})
	_ = fs.Parse(flagArgs)

	if *token == "" {
		if b, err := os.ReadFile(configDir() + "/remote-token"); err == nil {
			*token = strings.TrimSpace(string(b))
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	busURL := "ws://" + net.JoinHostPort(*host, fmt.Sprintf("%d", *hubPort)) + "/bus"
	cli := busclient.New(busURL, *token)
	go cli.Run(ctx)

	raw, err := cli.Call(ctx, "fleet.quiescence", map[string]any{})
	if err != nil {
		if errors.Is(err, busclient.ErrNotConnected) {
			err = fmt.Errorf("hub unreachable at %s:%d (is it running? wrong token?)", *host, *hubPort)
		}
		// Exit 2, never 1: a caller that cannot ask has not learned that the
		// fleet is busy, and conflating the two is how a script ends up acting
		// on an answer nobody gave it.
		fmt.Fprintf(os.Stderr, "workspacer fleet: %v\n", err)
		return 2
	}
	var ans quiescenceAnswer
	if err := json.Unmarshal(raw, &ans); err != nil {
		fmt.Fprintf(os.Stderr, "workspacer fleet: unreadable answer: %v\n", err)
		return 2
	}

	switch {
	case *quiet:
	case *jsonOut:
		fmt.Println(string(raw))
	default:
		fmt.Print(renderQuiescence(ans))
	}
	if ans.Quiescent {
		return 0
	}
	return 1
}

func renderQuiescence(a quiescenceAnswer) string {
	var b strings.Builder
	if a.Quiescent {
		b.WriteString("at rest")
		if a.Since != nil {
			b.WriteString(" since " + time.UnixMilli(*a.Since).Format(time.RFC3339))
		}
		b.WriteString("\n")
		return b.String()
	}
	fmt.Fprintf(&b, "not at rest (%d blocker(s)):\n", len(a.Blockers))
	for _, bl := range a.Blockers {
		if bl.ID != "" {
			fmt.Fprintf(&b, "  %-20s %s\n", bl.Kind, bl.ID)
			fmt.Fprintf(&b, "  %-20s %s\n", "", bl.Detail)
			continue
		}
		fmt.Fprintf(&b, "  %-20s %s\n", bl.Kind, bl.Detail)
	}
	return b.String()
}
