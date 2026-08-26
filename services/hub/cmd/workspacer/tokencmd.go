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
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

const tokenUsage = `workspacer token — capability-scoped bus tokens

Usage:
  workspacer token create --scope view|triage|operator|provider [--label <text>] [--full-access]
  workspacer token list
  workspacer token revoke <token-or-prefix>

Scopes:
  view      read-only: fleet lists, session snapshots, transcripts, event streams
  triage    view + acting on attention: approve/deny, answer, send message,
            interrupt, Web Push subscription (what the /m phone client needs;
            no spawn, no terminals, no git, no admin)
  operator  everything — equivalent to the pairing token "workspacer serve" prints
  provider  a headless capability PROVIDER (a remote node running "brain --hub"),
            not a rung on the ladder above: it may REGISTER capabilities and
            answer calls, and publish only the topics carrying the output of what
            it registered. It calls one method (layout.get), subscribes to
            nothing, and is refused nodes.wake/sleep, jobs.*, spawning, config,
            and the token-guarded HTTP routes including POST /plugins/install

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

// firstPositional returns the index of the first argument that is not one of the
// flags this command DECLARES, or -1 when there is none.
//
// It matches against the declared names rather than against a leading "-",
// because the positional argument here is a bus token and authtoken mints with
// base64.RawURLEncoding — whose alphabet includes '-'. "Starts with a dash" and
// "is a flag" are not the same question for this command.
//
// valueFlags names the flags that consume the NEXT argument; boolean flags do
// not, and `--name=value` carries its own.
func firstPositional(args []string, valueFlags map[string]bool) int {
	for i := 0; i < len(args); i++ {
		a := args[i]
		if a == "--" {
			return -1 // the caller already separated them
		}
		if !strings.HasPrefix(a, "-") || a == "-" {
			return i
		}
		name := strings.TrimLeft(a, "-")
		inline := false
		if k, _, ok := strings.Cut(name, "="); ok {
			name, inline = k, true
		}
		switch {
		case name == "h" || name == "help":
			// a declared boolean; consumes nothing
		case valueFlags[name]:
			if !inline {
				i++ // its value is the next argument
			}
		default:
			return i // not a flag this command knows: it is the token
		}
	}
	return -1
}

func runTokenCreate(args []string) int {
	fs := flag.NewFlagSet("workspacer token create", flag.ExitOnError)
	scopeFlag := fs.String("scope", "", "grant tier: view | triage | operator | provider (required)")
	label := fs.String("label", "", "human-readable label (e.g. \"dana's phone\")")
	// The full-access grant, mint-time only and never claimable by the holder.
	// Needed by a FEDERATION LINK above all: a peer link inherits no host trust
	// from being authenticated, so a peer that should be able to dispatch
	// full-access work needs a token that says so (internal/bus
	// conn.mayBypassPermissions).
	fullAccess := fs.Bool("full-access", false,
		"let agents spawned with this token skip approval prompts (--dangerously-skip-permissions)")
	path := tokensPathFlag(fs)
	_ = fs.Parse(args)

	if *scopeFlag == "" {
		fmt.Fprintln(os.Stderr, "workspacer token create: --scope is required (view | triage | operator | provider)")
		return 2
	}
	scope, err := authtoken.ParseScope(*scopeFlag)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer token create: %v\n", err)
		return 2
	}
	if *fullAccess && scope != authtoken.ScopeOperator {
		// view/triage cannot reach agents.spawn at all and a provider token may
		// only ANSWER calls, so the grant would be inert — and an inert security
		// flag that reports success is how someone concludes a link is granted
		// when it is not.
		fmt.Fprintf(os.Stderr, "workspacer token create: --full-access needs --scope operator (a %s token may not spawn at all)\n", scope)
		return 2
	}
	rec, err := authtoken.MintGranted(*path, scope, *label, *fullAccess)
	if err != nil {
		fmt.Fprintf(os.Stderr, "workspacer token create: %v\n", err)
		return 1
	}
	fmt.Printf("%s\n", rec.Token)
	fmt.Fprintf(os.Stderr, "minted %s token%s — connect with ?token=… or Authorization: Bearer …\n",
		rec.Scope, labelSuffix(rec.Label))
	if rec.YoloAllowed {
		fmt.Fprintln(os.Stderr,
			"  FULL ACCESS: agents spawned with this token may skip every approval prompt.\n"+
				"  Put it in the peer's peers.json entry to let that peer dispatch full-access work here.")
	}
	if rec.Scope == authtoken.ScopeProvider {
		// There is deliberately no --provides flag. A grant narrower than what
		// the provider registers puts the brain in a permanent 5s re-register
		// loop, because the `registered` ack carries no reason for a withheld
		// method — see authtoken.Record.Provides. So the mint is always the
		// full register surface, and the line below says what that does and
		// does not mean, since "provides: *" reads like the operator "*" and is
		// a different authority entirely.
		fmt.Fprintln(os.Stderr,
			"  it may REGISTER any capability and answer calls; it may not spawn, wake or sleep nodes,\n"+
				"  create jobs, read the event feed, or reach the token-guarded HTTP routes.")
	}
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
		fmt.Println("no scoped tokens (mint one with `workspacer token create --scope view|triage|operator|provider`)")
		return 0
	}
	fmt.Printf("%-34s  %-9s  %-20s  %s\n", "TOKEN", "SCOPE", "CREATED", "LABEL")
	for _, r := range recs {
		fmt.Printf("%-34s  %-9s  %-20s  %s\n", r.Token, r.Scope, r.Created.Format("2006-01-02 15:04:05"), r.Label)
	}
	return 0
}

func runTokenRevoke(args []string) int {
	fs := flag.NewFlagSet("workspacer token revoke", flag.ContinueOnError)
	path := tokensPathFlag(fs)
	// authtoken mints with base64.RawURLEncoding, whose alphabet includes '-',
	// so roughly one token in 64 STARTS with one — and a bare "-QWGMC1Ib9FK"
	// argument is read by the flag package as an unknown flag. With ExitOnError
	// that killed the process (and, inside `go test`, the test binary) instead of
	// revoking anything, which made revoking a leaked credential impossible for
	// exactly the tokens that need it. Everything after "--" is positional by
	// convention, so insert it before the first non-flag argument.
	if i := firstPositional(args, map[string]bool{"tokens-file": true}); i >= 0 {
		args = append(append(append([]string{}, args[:i]...), "--"), args[i:]...)
	}
	if err := fs.Parse(args); err != nil {
		return 2
	}

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
