package main

// `workspacer token …` — mint / list / revoke capability-scoped bus tokens.
//
// The host remote-token (printed by `workspacer serve`) is the operator
// pairing credential: full access, and every existing pairing keeps working
// with it. Scoped tokens are what you hand out when full access is too much —
// a `view` token for a read-only dashboard, a `triage` token for a phone that
// should approve/answer/chat but never spawn or touch git. They live in
// <config>/workspacer/tokens.json next to remote-token; the hub re-reads that
// file on each new connection, so create/revoke here take effect live.

import (
	"flag"
	"fmt"
	"os"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

const tokenUsage = `workspacer token — capability-scoped bus tokens

Usage:
  workspacer token create --scope view|triage|operator [--label <text>]
  workspacer token list
  workspacer token revoke <token-or-prefix>

Scopes:
  view      read-only: fleet lists, session snapshots, transcripts, event streams
  triage    view + acting on attention: approve/deny, answer, send message,
            interrupt, Web Push subscription (what the /m phone client needs;
            no spawn, no terminals, no git, no admin)
  operator  everything — equivalent to the pairing token "workspacer serve" prints

Tokens persist in <config>/workspacer/tokens.json (next to remote-token) and
take effect on the next connection — no server restart needed.
`

func runToken(args []string) int {
	if len(args) < 1 {
		fmt.Fprint(os.Stderr, tokenUsage)
		return 2
	}
	switch args[0] {
	case "create":
		return runTokenCreate(args[1:])
	case "list":
		return runTokenList(args[1:])
	case "revoke":
		return runTokenRevoke(args[1:])
	case "help", "-h", "--help":
		fmt.Print(tokenUsage)
		return 0
	default:
		fmt.Fprintf(os.Stderr, "workspacer token: unknown subcommand %q\n\n%s", args[0], tokenUsage)
		return 2
	}
}

// tokensPathFlag adds the shared --tokens-file flag so tests (and unusual
// setups) can aim at a different file than the live default.
func tokensPathFlag(fs *flag.FlagSet) *string {
	return fs.String("tokens-file", authtoken.DefaultPath(),
		"tokens file (default: <config>/workspacer/tokens.json, the one the hub reads)")
}

func runTokenCreate(args []string) int {
	fs := flag.NewFlagSet("workspacer token create", flag.ExitOnError)
	scopeFlag := fs.String("scope", "", "grant tier: view | triage | operator (required)")
	label := fs.String("label", "", "human-readable label (e.g. \"dana's phone\")")
	path := tokensPathFlag(fs)
	_ = fs.Parse(args)

	if *scopeFlag == "" {
		fmt.Fprintln(os.Stderr, "workspacer token create: --scope is required (view | triage | operator)")
		return 2
	}
	scope, err := authtoken.ParseScope(*scopeFlag)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer token create: %v\n", err)
		return 2
	}
	rec, err := authtoken.Mint(*path, scope, *label)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer token create: %v\n", err)
		return 1
	}
	fmt.Printf("%s\n", rec.Token)
	fmt.Fprintf(os.Stderr, "minted %s token%s — connect with ?token=… or Authorization: Bearer …\n",
		rec.Scope, labelSuffix(rec.Label))
	return 0
}

func runTokenList(args []string) int {
	fs := flag.NewFlagSet("workspacer token list", flag.ExitOnError)
	path := tokensPathFlag(fs)
	_ = fs.Parse(args)

	recs, err := authtoken.Load(*path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer token list: %v\n", err)
		return 1
	}
	if len(recs) == 0 {
		fmt.Println("no scoped tokens (mint one with `workspacer token create --scope view|triage|operator`)")
		return 0
	}
	fmt.Printf("%-34s  %-8s  %-20s  %s\n", "TOKEN", "SCOPE", "CREATED", "LABEL")
	for _, r := range recs {
		fmt.Printf("%-34s  %-8s  %-20s  %s\n", r.Token, r.Scope, r.Created.Format("2006-01-02 15:04:05"), r.Label)
	}
	return 0
}

func runTokenRevoke(args []string) int {
	fs := flag.NewFlagSet("workspacer token revoke", flag.ExitOnError)
	path := tokensPathFlag(fs)
	_ = fs.Parse(args)

	if fs.NArg() != 1 {
		fmt.Fprintln(os.Stderr, "workspacer token revoke: give exactly one token (or a unique ≥8-char prefix from `workspacer token list`)")
		return 2
	}
	rec, err := authtoken.Revoke(*path, fs.Arg(0))
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer token revoke: %v\n", err)
		return 1
	}
	fmt.Fprintf(os.Stderr, "revoked %s token%s — new connections with it are refused (already-open ones drop on disconnect)\n",
		rec.Scope, labelSuffix(rec.Label))
	return 0
}

func labelSuffix(label string) string {
	if label == "" {
		return ""
	}
	return fmt.Sprintf(" %q", label)
}
