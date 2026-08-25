package main

// `workspacer jobs …` — manage the hub's scheduled jobs from a terminal.
//
// This is the surface that closes the loop for a spec written somewhere else:
// an agent (or you, from the docs) writes the JSON, and `workspacer jobs add -f
// spec.json` installs it. Before it existed, an off-app spec could only be
// retyped into Settings → Jobs. Editing jobs.json directly is now a supported
// route too (the hub re-reads it on the scheduler tick), so this command is the
// convenience for a spec that lives in another file, and `jobs list` is how you
// confirm a hand edit landed.
//
// It holds HOST authority on purpose: it reads the same <config>/remote-token
// the desktop persists, so `jobs add` reaches jobs.upsert — the method the MCP
// facade deliberately withholds from agents. Approving an agent's proposal is
// therefore a thing a human does, whether in the UI or here.

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/jobs"
)

const jobsUsage = `workspacer jobs — scheduled jobs the hub runs for you

Usage:
  workspacer jobs list [--json]              list jobs with next run and last result
  workspacer jobs add -f <file>|-            install a job spec (JSON; - reads stdin)
  workspacer jobs show <id>                  print one job's spec as JSON
  workspacer jobs history <id> [--json]      recent runs for a job
  workspacer jobs run <id>                   run a job now
  workspacer jobs approve <id>               arm an agent's proposal (and enable it)
  workspacer jobs enable <id> | disable <id> flip a job without editing it
  workspacer jobs remove <id>                delete a job and its history

A spec is {"name","enabled","trigger","action"} — the shape documented at
landing/docs.html#jobs. "add" prints the id the hub minted; pass an existing id
in the spec to REPLACE that job.

Jobs proposed by an agent arrive disabled and marked (jobs list shows
"proposal"); they never run until "jobs approve" (or the Jobs UI) arms them.

Flags: --host (127.0.0.1), --hub-port (7895), --token ($HUB_TOKEN, else the
persisted <config>/workspacer/remote-token).
`

func runJobs(args []string) int {
	if len(args) < 1 {
		fmt.Fprint(os.Stderr, jobsUsage)
		return 2
	}
	switch args[0] {
	case "list":
		return runJobsList(args[1:])
	case "add":
		return runJobsAdd(args[1:])
	case "show":
		return runJobsShow(args[1:])
	case "history":
		return runJobsHistory(args[1:])
	case "run":
		return runJobsRun(args[1:])
	case "approve":
		return runJobsApprove(args[1:])
	case "enable":
		return runJobsSetEnabled(args[1:], true)
	case "disable":
		return runJobsSetEnabled(args[1:], false)
	case "remove":
		return runJobsRemove(args[1:])
	case "help", "-h", "--help":
		fmt.Print(jobsUsage)
		return 0
	default:
		fmt.Fprintf(os.Stderr, "workspacer jobs: unknown subcommand %q\n\n%s", args[0], jobsUsage)
		return 2
	}
}

// jobsFlags declares the connection flags every subcommand shares.
type jobsFlags struct {
	host    *string
	hubPort *int
	token   *string
	jsonOut *bool
}

// jobsValueFlags names the flags that take a value, for splitPositionals.
var jobsValueFlags = map[string]bool{"host": true, "hub-port": true, "token": true, "f": true}

// splitPositionals separates flags from positional arguments in EITHER order.
//
// Go's flag package stops parsing at the first positional, so
// `jobs enable <id> --hub-port 7896` parsed no flags at all and silently fell
// back to the default host, port and token — i.e. it went and operated on the
// DEFAULT hub while the user was clearly naming another one. Wrong-machine is
// the worst possible failure for a command that installs and runs argv, and it
// failed silently, so the argument order is normalized here instead.
func splitPositionals(args []string, valueFlags map[string]bool) (flags, positional []string) {
	for i := 0; i < len(args); i++ {
		a := args[i]
		if a == "--" {
			positional = append(positional, args[i+1:]...)
			return flags, positional
		}
		if !strings.HasPrefix(a, "-") || a == "-" {
			positional = append(positional, a)
			continue
		}
		flags = append(flags, a)
		name := strings.TrimLeft(a, "-")
		if k, _, ok := strings.Cut(name, "="); ok {
			name = k
			continue // value rode along inline
		}
		if valueFlags[name] && i+1 < len(args) {
			i++
			flags = append(flags, args[i])
		}
	}
	return flags, positional
}

// parseJobs parses the flags wherever they sit and returns the positionals.
func parseJobs(fs *flag.FlagSet, args []string) []string {
	flagArgs, positional := splitPositionals(args, jobsValueFlags)
	_ = fs.Parse(flagArgs)
	return positional
}

func jobsFlagSet(name string) (*flag.FlagSet, *jobsFlags) {
	fs := flag.NewFlagSet("workspacer jobs "+name, flag.ExitOnError)
	f := &jobsFlags{
		host:    fs.String("host", "127.0.0.1", "hub host"),
		hubPort: fs.Int("hub-port", 7895, "hub port"),
		token: fs.String("token", os.Getenv("HUB_TOKEN"),
			"bus auth token (default: $HUB_TOKEN, else the persisted <config>/workspacer/remote-token)"),
		jsonOut: fs.Bool("json", false, "print raw JSON"),
	}
	return fs, f
}

// dialJobs connects to the hub bus with host authority. Every jobs.* method is
// trusted-only, so a missing/wrong token surfaces as a plain refusal rather
// than a partial result.
func dialJobs(ctx context.Context, f *jobsFlags) (*busclient.Client, error) {
	token := *f.token
	if token == "" {
		if b, err := os.ReadFile(configDir() + "/remote-token"); err == nil {
			token = strings.TrimSpace(string(b))
		}
	}
	busURL := "ws://" + net.JoinHostPort(*f.host, fmt.Sprintf("%d", *f.hubPort)) + "/bus"
	cli := busclient.New(busURL, token)
	go cli.Run(ctx)
	return cli, nil
}

// callJobs runs one jobs.* method and returns its raw result.
func callJobs(f *jobsFlags, method string, params any) (json.RawMessage, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	cli, err := dialJobs(ctx, f)
	if err != nil {
		return nil, err
	}
	res, err := cli.Call(ctx, method, params)
	if err != nil {
		if errors.Is(err, busclient.ErrNotConnected) {
			return nil, fmt.Errorf("hub unreachable at %s:%d (is it running? wrong token?)", *f.host, *f.hubPort)
		}
		return nil, err
	}
	return res, nil
}

func jobsFail(err error) int {
	fmt.Fprintf(os.Stderr, "workspacer jobs: %v\n", err)
	return 1
}

// jobView mirrors the jobs.list row: the spec plus live scheduling state.
type jobView struct {
	jobs.Job
	NextRunAt int64     `json:"nextRunAt,omitempty"`
	LastRun   *jobs.Run `json:"lastRun,omitempty"`
	Running   bool      `json:"running,omitempty"`
}

func fetchJobs(f *jobsFlags) ([]jobView, json.RawMessage, error) {
	raw, err := callJobs(f, "jobs.list", map[string]any{})
	if err != nil {
		return nil, nil, err
	}
	var doc struct {
		Jobs []jobView `json:"jobs"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, raw, fmt.Errorf("unreadable jobs.list answer: %w", err)
	}
	return doc.Jobs, raw, nil
}

func runJobsList(args []string) int {
	fs, f := jobsFlagSet("list")
	parseJobs(fs, args)
	views, raw, err := fetchJobs(f)
	if err != nil {
		return jobsFail(err)
	}
	if *f.jsonOut {
		fmt.Println(string(raw))
		return 0
	}
	if len(views) == 0 {
		fmt.Println("no jobs yet — `workspacer jobs add -f spec.json` installs one")
		return 0
	}
	for _, v := range views {
		fmt.Printf("%s  %s\n", v.ID, v.Name)
		fmt.Printf("    %s · %s\n", triggerLine(v.Job), actionLine(v.Job))
		fmt.Printf("    %s\n", stateLine(v))
	}
	return 0
}

// triggerLine renders a trigger the way the UI's summary does.
func triggerLine(j jobs.Job) string {
	t := j.Trigger
	switch t.Kind {
	case "interval":
		if t.EveryMinutes%60 == 0 && t.EveryMinutes > 0 {
			return fmt.Sprintf("every %dh", t.EveryMinutes/60)
		}
		return fmt.Sprintf("every %dm", t.EveryMinutes)
	case "daily":
		if len(t.Days) > 0 {
			names := make([]string, 0, len(t.Days))
			for _, d := range t.Days {
				names = append(names, [...]string{"Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"}[d%7])
			}
			return "daily " + t.At + " · " + strings.Join(names, " ")
		}
		return "daily " + t.At
	case "once":
		return "once, " + t.Once
	default:
		return "manual"
	}
}

func actionLine(j jobs.Job) string {
	switch j.Action.Kind {
	case "spawn":
		steps := ""
		if n := len(j.Action.Spawn.Context); n > 0 {
			steps = fmt.Sprintf("%d step%s → ", n, plural(n))
		}
		return steps + "agent in " + j.Action.Spawn.Cwd
	case "shell":
		return "$ " + j.Action.Shell.Command
	case "call":
		return "call " + j.Action.Call.Method
	}
	return j.Action.Kind
}

func plural(n int) string {
	if n == 1 {
		return ""
	}
	return "s"
}

// stateLine is the one line that answers "is this thing going to run, and how
// did it go last time" — including the proposal state, which is the whole
// reason an agent-written job doesn't just quietly start firing.
func stateLine(v jobView) string {
	var parts []string
	switch {
	case v.IsProposal():
		parts = append(parts, fmt.Sprintf("PROPOSAL by %s — needs `workspacer jobs approve %s`", v.ProposedBy, v.ID))
	case !v.Enabled:
		parts = append(parts, "disabled")
	case v.Running:
		parts = append(parts, "running now")
	case v.NextRunAt > 0:
		parts = append(parts, "next "+time.UnixMilli(v.NextRunAt).Format("Mon 15:04"))
	default:
		parts = append(parts, "no next run")
	}
	if v.LastRun != nil {
		detail := strings.ReplaceAll(strings.TrimSpace(v.LastRun.Detail), "\n", " ")
		if len(detail) > 70 {
			detail = detail[:70] + "…"
		}
		last := "last " + v.LastRun.Status
		if detail != "" {
			last += ": " + detail
		}
		parts = append(parts, last)
	}
	return strings.Join(parts, " · ")
}

// readSpec loads a job spec from a file or stdin ("-").
func readSpec(path string) (jobs.Job, error) {
	var j jobs.Job
	var raw []byte
	var err error
	if path == "-" {
		raw, err = io.ReadAll(os.Stdin)
	} else {
		raw, err = os.ReadFile(path)
	}
	if err != nil {
		return j, err
	}
	if err := json.Unmarshal(raw, &j); err != nil {
		return j, fmt.Errorf("%s is not a valid job spec: %w", path, err)
	}
	// Validate locally too: a malformed spec should say so before it becomes a
	// bus round-trip, and the message is the same one the hub would give.
	if err := jobs.Validate(&j); err != nil {
		return j, err
	}
	return j, nil
}

func runJobsAdd(args []string) int {
	fs, f := jobsFlagSet("add")
	file := fs.String("f", "", "job spec file, or - for stdin")
	parseJobs(fs, args)
	if *file == "" {
		fmt.Fprint(os.Stderr, "workspacer jobs add: -f <file> (or -f - for stdin) is required\n")
		return 2
	}
	j, err := readSpec(*file)
	if err != nil {
		return jobsFail(err)
	}
	raw, err := callJobs(f, "jobs.upsert", j)
	if err != nil {
		return jobsFail(err)
	}
	var saved jobs.Job
	_ = json.Unmarshal(raw, &saved)
	if *f.jsonOut {
		fmt.Println(string(raw))
		return 0
	}
	verb := "installed"
	if !saved.Enabled {
		verb = "installed (disabled)"
	}
	fmt.Printf("%s %s — %s · %s\n", verb, saved.ID, triggerLine(saved), actionLine(saved))
	return 0
}

// findJob resolves an id or unique id prefix, so a human never has to paste 24
// hex characters exactly.
func findJob(f *jobsFlags, idOrPrefix string) (jobView, error) {
	views, _, err := fetchJobs(f)
	if err != nil {
		return jobView{}, err
	}
	var hits []jobView
	for _, v := range views {
		if v.ID == idOrPrefix {
			return v, nil
		}
		if strings.HasPrefix(v.ID, idOrPrefix) {
			hits = append(hits, v)
		}
	}
	switch len(hits) {
	case 1:
		return hits[0], nil
	case 0:
		return jobView{}, fmt.Errorf("no job matching %q (try `workspacer jobs list`)", idOrPrefix)
	default:
		return jobView{}, fmt.Errorf("%q matches %d jobs — use more of the id", idOrPrefix, len(hits))
	}
}

// oneArg pulls the single positional id every id-taking subcommand needs.
func oneArg(positional []string, name string) (string, bool) {
	if len(positional) < 1 {
		fmt.Fprintf(os.Stderr, "workspacer jobs %s: an id is required\n", name)
		return "", false
	}
	return positional[0], true
}

func runJobsShow(args []string) int {
	fs, f := jobsFlagSet("show")
	id, ok := oneArg(parseJobs(fs, args), "show")
	if !ok {
		return 2
	}
	v, err := findJob(f, id)
	if err != nil {
		return jobsFail(err)
	}
	out, _ := json.MarshalIndent(v.Job, "", "  ")
	fmt.Println(string(out))
	return 0
}

func runJobsHistory(args []string) int {
	fs, f := jobsFlagSet("history")
	id, ok := oneArg(parseJobs(fs, args), "history")
	if !ok {
		return 2
	}
	v, err := findJob(f, id)
	if err != nil {
		return jobsFail(err)
	}
	raw, err := callJobs(f, "jobs.history", map[string]any{"id": v.ID})
	if err != nil {
		return jobsFail(err)
	}
	if *f.jsonOut {
		fmt.Println(string(raw))
		return 0
	}
	var doc struct {
		Runs []jobs.Run `json:"runs"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return jobsFail(err)
	}
	if len(doc.Runs) == 0 {
		fmt.Printf("%s has never run\n", v.Name)
		return 0
	}
	for _, r := range doc.Runs {
		detail := strings.ReplaceAll(strings.TrimSpace(r.Detail), "\n", " ")
		fmt.Printf("%s  %-8s %s\n", time.UnixMilli(r.StartedAt).Format("2006-01-02 15:04"), r.Status, detail)
	}
	return 0
}

func runJobsRun(args []string) int {
	fs, f := jobsFlagSet("run")
	id, ok := oneArg(parseJobs(fs, args), "run")
	if !ok {
		return 2
	}
	v, err := findJob(f, id)
	if err != nil {
		return jobsFail(err)
	}
	raw, err := callJobs(f, "jobs.run", map[string]any{"id": v.ID})
	if err != nil {
		return jobsFail(err)
	}
	var res struct {
		Started bool   `json:"started"`
		Reason  string `json:"reason"`
	}
	_ = json.Unmarshal(raw, &res)
	if !res.Started {
		fmt.Printf("not started: %s\n", res.Reason)
		return 1
	}
	fmt.Printf("running %s — `workspacer jobs history %s` for the result\n", v.Name, v.ID)
	return 0
}

// runJobsApprove is the human half of the propose/approve split: it clears the
// proposal stamp (which is what makes the row runnable at all) and enables it.
func runJobsApprove(args []string) int {
	fs, f := jobsFlagSet("approve")
	keepDisabled := fs.Bool("disabled", false, "approve without enabling (arm it later)")
	id, ok := oneArg(parseJobs(fs, args), "approve")
	if !ok {
		return 2
	}
	v, err := findJob(f, id)
	if err != nil {
		return jobsFail(err)
	}
	if !v.IsProposal() {
		fmt.Printf("%s is not a proposal — nothing to approve\n", v.Name)
		return 0
	}
	j := v.Job
	j.ProposedBy = ""
	j.Enabled = !*keepDisabled
	if _, err := callJobs(f, "jobs.upsert", j); err != nil {
		return jobsFail(err)
	}
	state := "enabled"
	if *keepDisabled {
		state = "approved but left disabled"
	}
	fmt.Printf("approved %s (%s) — %s · %s\n", v.Name, state, triggerLine(j), actionLine(j))
	return 0
}

func runJobsSetEnabled(args []string, enabled bool) int {
	name := "disable"
	if enabled {
		name = "enable"
	}
	fs, f := jobsFlagSet(name)
	id, ok := oneArg(parseJobs(fs, args), name)
	if !ok {
		return 2
	}
	v, err := findJob(f, id)
	if err != nil {
		return jobsFail(err)
	}
	if enabled && v.IsProposal() {
		// Enabling a proposal through the back door would skip the review this
		// whole split exists for — approve is the one door.
		return jobsFail(fmt.Errorf("%s is an unapproved proposal — use `workspacer jobs approve %s`", v.Name, v.ID))
	}
	j := v.Job
	j.Enabled = enabled
	if _, err := callJobs(f, "jobs.upsert", j); err != nil {
		return jobsFail(err)
	}
	fmt.Printf("%sd %s\n", name, v.Name)
	return 0
}

func runJobsRemove(args []string) int {
	fs, f := jobsFlagSet("remove")
	id, ok := oneArg(parseJobs(fs, args), "remove")
	if !ok {
		return 2
	}
	v, err := findJob(f, id)
	if err != nil {
		return jobsFail(err)
	}
	if _, err := callJobs(f, "jobs.remove", map[string]any{"id": v.ID}); err != nil {
		return jobsFail(err)
	}
	fmt.Printf("removed %s\n", v.Name)
	return 0
}
