// Package jobs owns the hub's job system: recurring and one-off tasks the hub
// runs on the user's behalf — spawn an agent with a prompt, call a bus
// capability, or run a shell command — on an interval, at a daily time, once
// at a timestamp, or manually.
//
// The hub is the right home for the same reason it owns the layout: it is the
// always-on control plane, alive in desktop mode and `workspacer serve` alike.
// Execution stays deliberately dumb: spawn/call actions go BACK OUT over the
// bus through a self-dialed client, so `agents.spawn` is answered by whoever
// provides it (desktop main or the headless brain) and every clamp that path
// enforces on bus callers — no permission bypass, no mcpItemIds, profile
// configDir scrubbed — applies to jobs automatically.
//
// SECURITY: a persisted job is persisted argv (the scrubBypassProfile lesson),
// so the spec lives in a hub-owned 0600 file, NOT the library (agent-writable
// by design) and NOT the layout (world-readable, client-broadcast). Every
// jobs.* RPC is registered trusted-only in cmd/hub: the host token or an
// operator-tier pairing may manage jobs; plugin tokens and view/triage tokens
// are refused — a plugin manifest may still DECLARE jobs.*, but the identity
// gate refuses it at call time. Job events publish nothing of their own (a new
// topic namespace costs four pinned registries); failures ride the existing
// `notify.post` event, which both the desktop notification center and web
// clients already ingest.
package jobs

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/event"
)

// Trigger says when a job fires. Exactly one shape per Kind:
//   - "interval": every EveryMinutes minutes;
//   - "daily":    at At ("15:04", hub-local time) on Days (0=Sunday…6;
//     empty = every day);
//   - "once":     at Once (RFC3339); the job disables itself when it fires;
//   - "manual":   only via jobs.run.
//
// A due time that passes while the machine sleeps fires once on wake (the
// next tick sees it overdue) and then reanchors from now — no catch-up storm.
type Trigger struct {
	Kind         string `json:"kind"`
	EveryMinutes int    `json:"everyMinutes,omitempty"`
	At           string `json:"at,omitempty"`
	Days         []int  `json:"days,omitempty"`
	Once         string `json:"once,omitempty"`
}

// SpawnAction starts an agent and sends it a prompt. The spawn goes through
// the bus `agents.spawn` (clamped like any bus caller) and the prompt through
// `agents.sendMessage` — claudemon buffers a message sent before the session
// is ready, so the pair works from a cold start.
//
// Context runs BEFORE any of that: see ContextStep.
type SpawnAction struct {
	Cwd    string `json:"cwd"`
	Prompt string `json:"prompt"`
	// Context gathers material for the prompt before the agent exists — and
	// is the job's chance to decide there's nothing worth waking a model for.
	Context        []ContextStep `json:"context,omitempty"`
	Provider       string        `json:"provider,omitempty"`
	Model          string        `json:"model,omitempty"`
	Effort         string        `json:"effort,omitempty"`
	PermissionMode string        `json:"permissionMode,omitempty"`
}

// ContextStep runs code and feeds the result to the model: a shell command or
// a bus call whose output is substituted into the spawn prompt at `{{output}}`
// (or `{{output.N}}`, 1-based, when several steps run; a prompt with no
// placeholder gets the outputs appended as fenced blocks).
//
// Its second job is the more valuable one: a step can ABORT the run, so the
// guard is evaluated by a shell command rather than by a model reading output
// only to answer "nothing to do". Nothing is spawned until every step has run
// and every guard has passed — an aborted run records `skipped`, spends no
// tokens, and (unlike an error) raises no notification.
//
//	SkipIfEmpty     — no output means nothing happened; don't spawn.
//	SkipUnlessMatch — spawn only when the output matches this regexp.
//	IgnoreExitCode  — a nonzero exit is data, not failure (`grep` finding
//	                  nothing is the canonical guard). Only an *exit code* is
//	                  forgiven: a timeout or an unstartable command still fails.
//
// SECURITY: a context step is the same argv a `shell`/`call` action already
// persists, run by the same hub process — it adds reach to a spawn job, not
// privilege to a caller, and every jobs.* RPC stays trusted-only. Call steps
// are held to the same method rules as a call action (no `jobs.*` recursion,
// no `hub:<peer>/` hop to another machine).
type ContextStep struct {
	Kind  string       `json:"kind"` // "shell" | "call"
	Shell *ShellAction `json:"shell,omitempty"`
	Call  *CallAction  `json:"call,omitempty"`

	SkipIfEmpty     bool   `json:"skipIfEmpty,omitempty"`
	SkipUnlessMatch string `json:"skipUnlessMatch,omitempty"`
	IgnoreExitCode  bool   `json:"ignoreExitCode,omitempty"`
}

// CallAction invokes one bus capability with fixed params.
type CallAction struct {
	Method string          `json:"method"`
	Params json.RawMessage `json:"params,omitempty"`
}

// ShellAction runs a command through the host shell, from Cwd when set.
type ShellAction struct {
	Command string `json:"command"`
	Cwd     string `json:"cwd,omitempty"`
}

// Action is what a job does; exactly one of the pointers matches Kind
// ("spawn" | "call" | "shell").
type Action struct {
	Kind  string       `json:"kind"`
	Spawn *SpawnAction `json:"spawn,omitempty"`
	Call  *CallAction  `json:"call,omitempty"`
	Shell *ShellAction `json:"shell,omitempty"`
}

// Job is one persisted spec. Timestamps are unix milliseconds.
type Job struct {
	ID      string  `json:"id"`
	Name    string  `json:"name"`
	Enabled bool    `json:"enabled"`
	Trigger Trigger `json:"trigger"`
	Action  Action  `json:"action"`
	// ProposedBy names the agent that asked for this job and, by being set at
	// all, marks the row as a PROPOSAL: something an agent wrote, that no human
	// has armed. A proposal never fires (the tick skips it and Propose forces
	// Enabled false), jobs.run refuses it, and it stays that way until a
	// trusted caller — the Jobs UI's Approve button, or the CLI — writes it
	// back with this field cleared.
	//
	// The threat it answers: a job is argv that runs unattended, forever. An
	// operator-tier agent already holds bus authority (its token matches `*`),
	// so the restraint can't come from the identity gate — it comes from the
	// METHOD: the facade exposes jobs.propose and never jobs.upsert, and
	// jobs.propose disarms whatever it's handed. One injected transcript
	// therefore buys a suggestion in a list, not a nightly backdoor.
	ProposedBy string `json:"proposedBy,omitempty"`
	CreatedAt  int64  `json:"createdAt"`
	UpdatedAt  int64  `json:"updatedAt"`
}

// IsProposal reports whether a human has yet to arm this job.
func (j *Job) IsProposal() bool { return strings.TrimSpace(j.ProposedBy) != "" }

// Run is one execution record. Status: "ok" | "error" | "skipped" (a due fire
// found the previous run still going). Detail holds a result/output tail or
// the error string.
type Run struct {
	JobID      string `json:"jobId"`
	StartedAt  int64  `json:"startedAt"`
	FinishedAt int64  `json:"finishedAt,omitempty"`
	Status     string `json:"status"`
	Detail     string `json:"detail,omitempty"`
}

// Runner executes a job's action. Split from the Service so the scheduler is
// testable without a bus or a shell.
type Runner interface {
	// Call invokes a bus capability (via the hub's self-dialed client).
	Call(ctx context.Context, method string, params any) (json.RawMessage, error)
	// Shell runs a command through the host shell, returning combined output.
	Shell(ctx context.Context, command, cwd string) (string, error)
}

const (
	maxRunsPerJob = 30
	runTimeout    = 15 * time.Minute
	// detailCap bounds what a run record keeps of output/results.
	detailCap = 2000
	// maxContextSteps bounds the pre-spawn work: the steps share the job's
	// single runTimeout, so an unbounded list is an unbounded stall in front
	// of an agent that hasn't spawned yet.
	maxContextSteps = 4
	// maxPendingProposals bounds unapproved agent proposals. Without it, a
	// looping agent turns the Jobs list into a junk drawer nobody reads — and
	// a review queue nobody reads is a review queue that gets approved blind.
	maxPendingProposals = 20
	// contextCap bounds what ONE step contributes to the prompt. The prompt is
	// a message, and the model pays for every character of it — a `git log`
	// that grew unattended shouldn't quietly become the whole context window.
	contextCap = 12000
)

func newID() string {
	b := make([]byte, 12)
	_, _ = rand.Read(b)
	return hex.EncodeToString(b)
}

// Validate rejects a malformed job. This is the authoritative check — the
// desktop form mirrors it loosely, but the hub owns the file.
func Validate(j *Job) error {
	if strings.TrimSpace(j.Name) == "" {
		return errors.New("job needs a name")
	}
	switch j.Trigger.Kind {
	case "interval":
		if j.Trigger.EveryMinutes < 1 {
			return errors.New("interval trigger needs everyMinutes >= 1")
		}
	case "daily":
		if _, err := time.Parse("15:04", j.Trigger.At); err != nil {
			return fmt.Errorf("daily trigger needs at as HH:MM: %w", err)
		}
		for _, d := range j.Trigger.Days {
			if d < 0 || d > 6 {
				return fmt.Errorf("daily trigger day out of range: %d", d)
			}
		}
	case "once":
		if _, err := time.Parse(time.RFC3339, j.Trigger.Once); err != nil {
			return fmt.Errorf("once trigger needs an RFC3339 time: %w", err)
		}
	case "manual":
		// nothing to check
	default:
		return fmt.Errorf("unknown trigger kind %q", j.Trigger.Kind)
	}
	switch j.Action.Kind {
	case "spawn":
		if j.Action.Spawn == nil || strings.TrimSpace(j.Action.Spawn.Cwd) == "" ||
			strings.TrimSpace(j.Action.Spawn.Prompt) == "" {
			return errors.New("spawn action needs cwd and prompt")
		}
		if err := validateContext(j.Action.Spawn.Context); err != nil {
			return err
		}
	case "call":
		if j.Action.Call == nil || strings.TrimSpace(j.Action.Call.Method) == "" {
			return errors.New("call action needs a method")
		}
		if err := validateCallMethod(j.Action.Call.Method); err != nil {
			return err
		}
	case "shell":
		if j.Action.Shell == nil || strings.TrimSpace(j.Action.Shell.Command) == "" {
			return errors.New("shell action needs a command")
		}
	default:
		return fmt.Errorf("unknown action kind %q", j.Action.Kind)
	}
	return nil
}

// validateCallMethod holds a bus call — an action's or a context step's — to
// the two methods jobs may not reach: jobs.* would recurse into this very
// surface, and hub:<peer>/ would run the call on ANOTHER machine (jobs are
// host-local by design, and job state deliberately doesn't federate).
func validateCallMethod(method string) error {
	if strings.HasPrefix(method, "jobs.") || strings.HasPrefix(method, "hub:") {
		return fmt.Errorf("call may not target %q", method)
	}
	return nil
}

func validateContext(steps []ContextStep) error {
	if len(steps) > maxContextSteps {
		return fmt.Errorf("at most %d context steps (got %d)", maxContextSteps, len(steps))
	}
	for i, st := range steps {
		n := i + 1
		switch st.Kind {
		case "shell":
			if st.Shell == nil || strings.TrimSpace(st.Shell.Command) == "" {
				return fmt.Errorf("context step %d needs a command", n)
			}
		case "call":
			if st.Call == nil || strings.TrimSpace(st.Call.Method) == "" {
				return fmt.Errorf("context step %d needs a method", n)
			}
			if err := validateCallMethod(st.Call.Method); err != nil {
				return fmt.Errorf("context step %d: %w", n, err)
			}
		default:
			return fmt.Errorf("context step %d has unknown kind %q", n, st.Kind)
		}
		if st.SkipUnlessMatch != "" {
			// Compiled here so a bad pattern is refused at save time rather
			// than silently never matching (which reads as "the job stopped
			// running") at 3am.
			if _, err := regexp.Compile(st.SkipUnlessMatch); err != nil {
				return fmt.Errorf("context step %d has an invalid skipUnlessMatch: %w", n, err)
			}
		}
	}
	return nil
}

// NextRun is the earliest fire time strictly after `after` for a trigger, in
// `loc`. ok=false for manual triggers (and unparseable specs, which Validate
// should have refused).
func NextRun(t Trigger, after time.Time, loc *time.Location) (time.Time, bool) {
	switch t.Kind {
	case "interval":
		if t.EveryMinutes < 1 {
			return time.Time{}, false
		}
		return after.Add(time.Duration(t.EveryMinutes) * time.Minute), true
	case "daily":
		hm, err := time.Parse("15:04", t.At)
		if err != nil {
			return time.Time{}, false
		}
		allowed := func(d time.Weekday) bool {
			if len(t.Days) == 0 {
				return true
			}
			for _, x := range t.Days {
				if x == int(d) {
					return true
				}
			}
			return false
		}
		base := after.In(loc)
		for d := 0; d < 8; d++ {
			day := base.AddDate(0, 0, d)
			cand := time.Date(day.Year(), day.Month(), day.Day(), hm.Hour(), hm.Minute(), 0, 0, loc)
			if cand.After(after) && allowed(cand.Weekday()) {
				return cand, true
			}
		}
		return time.Time{}, false // unreachable with a sane Days list
	case "once":
		at, err := time.Parse(time.RFC3339, t.Once)
		if err != nil {
			return time.Time{}, false
		}
		// A past time still fires (once, immediately): "run this at 9" set at
		// 9:05, or a laptop asleep at the appointed hour, should not silently
		// never run.
		return at, true
	default:
		return time.Time{}, false
	}
}

// Service holds the specs, persists them, schedules fires, and answers the
// jobs.* RPCs (identity-gated at the registration site in cmd/hub).
type Service struct {
	mu       sync.Mutex
	jobs     []Job
	history  map[string][]Run // newest first, capped at maxRunsPerJob
	nextAt   map[string]time.Time
	running  map[string]bool
	path     string
	histPath string
	b        *broker.Broker
	runner   Runner
	loc      *time.Location
	now      func() time.Time
}

// New builds the service, seeding jobs and run history from disk. path and
// histPath name the hub-owned 0600 files ("" disables persistence — tests).
func New(b *broker.Broker, path, histPath string, runner Runner) *Service {
	s := &Service{
		history:  map[string][]Run{},
		nextAt:   map[string]time.Time{},
		running:  map[string]bool{},
		path:     path,
		histPath: histPath,
		b:        b,
		runner:   runner,
		loc:      time.Local,
		now:      time.Now,
	}
	s.load()
	s.loadHistory()
	for i := range s.jobs {
		s.rescheduleLocked(&s.jobs[i])
	}
	return s
}

func (s *Service) load() {
	if s.path == "" {
		return
	}
	raw, err := os.ReadFile(s.path)
	if err != nil {
		return // first run
	}
	var doc struct {
		Jobs []Job `json:"jobs"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		log.Printf("[jobs] %s unreadable, starting empty: %v", s.path, err)
		return
	}
	for _, j := range doc.Jobs {
		if err := Validate(&j); err != nil {
			log.Printf("[jobs] dropping invalid persisted job %q: %v", j.Name, err)
			continue
		}
		s.jobs = append(s.jobs, j)
	}
}

func (s *Service) loadHistory() {
	if s.histPath == "" {
		return
	}
	raw, err := os.ReadFile(s.histPath)
	if err != nil {
		return
	}
	var doc struct {
		Runs map[string][]Run `json:"runs"`
	}
	if err := json.Unmarshal(raw, &doc); err == nil && doc.Runs != nil {
		s.history = doc.Runs
	}
}

// saveLocked persists the specs: 0600 (prompts and shell commands are the
// user's business), atomic tmp+rename like the layout store.
func (s *Service) saveLocked() {
	if s.path == "" {
		return
	}
	raw, err := json.MarshalIndent(struct {
		Jobs []Job `json:"jobs"`
	}{Jobs: s.jobs}, "", "  ")
	if err != nil {
		return
	}
	s.writeAtomic(s.path, raw)
}

func (s *Service) saveHistoryLocked() {
	if s.histPath == "" {
		return
	}
	raw, err := json.Marshal(struct {
		Runs map[string][]Run `json:"runs"`
	}{Runs: s.history})
	if err != nil {
		return
	}
	s.writeAtomic(s.histPath, raw)
}

func (s *Service) writeAtomic(path string, raw []byte) {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		log.Printf("[jobs] mkdir for %s: %v", path, err)
		return
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		log.Printf("[jobs] write %s: %v", path, err)
		return
	}
	if err := os.Rename(tmp, path); err != nil {
		log.Printf("[jobs] rename %s: %v", path, err)
	}
}

// rescheduleLocked recomputes a job's next fire time from now.
func (s *Service) rescheduleLocked(j *Job) {
	// A proposal is checked as well as the enabled flag: a hand-edited (or
	// future-bug) row carrying enabled:true with proposedBy still set must not
	// acquire a next run. Disarmed is a property of the row, not of one writer.
	if !j.Enabled || j.IsProposal() {
		delete(s.nextAt, j.ID)
		return
	}
	if at, ok := NextRun(j.Trigger, s.now(), s.loc); ok {
		s.nextAt[j.ID] = at
	} else {
		delete(s.nextAt, j.ID)
	}
}

// JobView is what jobs.list returns per job: the spec plus live scheduling
// state.
type JobView struct {
	Job
	NextRunAt int64 `json:"nextRunAt,omitempty"`
	LastRun   *Run  `json:"lastRun,omitempty"`
	Running   bool  `json:"running,omitempty"`
}

// List answers jobs.list.
func (s *Service) List(json.RawMessage) (any, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	views := make([]JobView, 0, len(s.jobs))
	for _, j := range s.jobs {
		v := JobView{Job: j, Running: s.running[j.ID]}
		if at, ok := s.nextAt[j.ID]; ok {
			v.NextRunAt = at.UnixMilli()
		}
		if runs := s.history[j.ID]; len(runs) > 0 {
			r := runs[0]
			v.LastRun = &r
		}
		views = append(views, v)
	}
	return map[string]any{"jobs": views}, nil
}

// Upsert answers jobs.upsert: create (empty id) or replace a job.
func (s *Service) Upsert(p json.RawMessage) (any, error) {
	var j Job
	if err := json.Unmarshal(p, &j); err != nil {
		return nil, fmt.Errorf("bad job: %w", err)
	}
	if err := Validate(&j); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	nowMs := s.now().UnixMilli()
	j.UpdatedAt = nowMs
	if j.ID == "" {
		j.ID = newID()
		j.CreatedAt = nowMs
		s.jobs = append(s.jobs, j)
	} else {
		found := false
		for i := range s.jobs {
			if s.jobs[i].ID == j.ID {
				j.CreatedAt = s.jobs[i].CreatedAt
				s.jobs[i] = j
				found = true
				break
			}
		}
		if !found {
			return nil, fmt.Errorf("no job %q", j.ID)
		}
	}
	s.rescheduleLocked(&j)
	s.saveLocked()
	return j, nil
}

// Propose answers jobs.propose: the AGENT-facing write. It is Upsert with the
// teeth pulled — the row lands disabled and stamped ProposedBy, so it sits in
// the Jobs list as a suggestion until a human arms it. Creates only: a
// proposal may not overwrite an existing job (that would let an agent edit an
// already-approved job's argv, which is the same escalation by another route).
func (s *Service) Propose(p json.RawMessage) (any, error) {
	var req struct {
		Job
		// ProposedBy is taken from the request because the hub can't see which
		// agent is behind a bus connection; it's a LABEL for the human
		// reviewing, never a permission. An empty one still marks a proposal.
		ProposedBy string `json:"proposedBy"`
	}
	if err := json.Unmarshal(p, &req); err != nil {
		return nil, fmt.Errorf("bad job: %w", err)
	}
	j := req.Job
	if strings.TrimSpace(req.ProposedBy) != "" {
		j.ProposedBy = req.ProposedBy
	}
	if strings.TrimSpace(j.ProposedBy) == "" {
		j.ProposedBy = "an agent"
	}
	// Disarmed here, not by the caller's good manners.
	j.Enabled = false
	j.ID = ""
	if err := Validate(&j); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	pending := 0
	for i := range s.jobs {
		if s.jobs[i].IsProposal() {
			pending++
		}
	}
	if pending >= maxPendingProposals {
		return nil, fmt.Errorf("%d job proposals are already waiting for review — approve or remove some first", pending)
	}
	nowMs := s.now().UnixMilli()
	j.ID = newID()
	j.CreatedAt = nowMs
	j.UpdatedAt = nowMs
	s.jobs = append(s.jobs, j)
	// Deliberately NOT rescheduled: a proposal has no next run.
	s.saveLocked()
	if s.b != nil {
		// Ride the existing notify.post — a proposal nobody hears about is a
		// proposal nobody reviews.
		s.b.Publish(event.New("notify.post", "jobs", map[string]any{
			"title": "Job proposed: " + j.Name,
			"body":  j.ProposedBy + " suggested a job. Review it in Settings → Jobs; it won't run until you approve it.",
			"level": "info",
			"key":   "job-proposal-" + j.ID,
		}))
	}
	return j, nil
}

// Remove answers jobs.remove.
func (s *Service) Remove(p json.RawMessage) (any, error) {
	var req struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(p, &req); err != nil || req.ID == "" {
		return nil, errors.New("jobs.remove requires {id}")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	kept := s.jobs[:0]
	for _, j := range s.jobs {
		if j.ID != req.ID {
			kept = append(kept, j)
		}
	}
	s.jobs = kept
	delete(s.nextAt, req.ID)
	delete(s.history, req.ID)
	s.saveLocked()
	s.saveHistoryLocked()
	return map[string]any{"ok": true}, nil
}

// RunNow answers jobs.run: fire a job immediately (any trigger kind).
func (s *Service) RunNow(p json.RawMessage) (any, error) {
	var req struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(p, &req); err != nil || req.ID == "" {
		return nil, errors.New("jobs.run requires {id}")
	}
	s.mu.Lock()
	var job *Job
	for i := range s.jobs {
		if s.jobs[i].ID == req.ID {
			job = &s.jobs[i]
			break
		}
	}
	if job == nil {
		s.mu.Unlock()
		return nil, fmt.Errorf("no job %q", req.ID)
	}
	if job.IsProposal() {
		s.mu.Unlock()
		// Otherwise "propose" and "run" compose into "install and execute",
		// and the review step was decoration.
		return nil, fmt.Errorf("job %q is an unapproved proposal — approve it in Settings → Jobs first", job.Name)
	}
	if s.running[req.ID] {
		s.mu.Unlock()
		return map[string]any{"started": false, "reason": "already running"}, nil
	}
	s.running[req.ID] = true
	j := *job
	s.mu.Unlock()
	go s.execute(j)
	return map[string]any{"started": true}, nil
}

// History answers jobs.history.
func (s *Service) History(p json.RawMessage) (any, error) {
	var req struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(p, &req); err != nil || req.ID == "" {
		return nil, errors.New("jobs.history requires {id}")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	runs := s.history[req.ID]
	out := make([]Run, len(runs))
	copy(out, runs)
	return map[string]any{"runs": out}, nil
}
