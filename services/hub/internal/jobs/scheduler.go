// The scheduling half of the jobs service: a coarse tick finds due jobs and
// executes them, one concurrent run per job (a due fire that lands mid-run is
// recorded as "skipped", never queued — agents run long, and a backlog of
// queued fires is a quota-burning failure mode, not a feature).
package jobs

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os/exec"
	"regexp"
	"runtime"
	"strings"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// tickEvery is deliberately coarse: minute-level triggers, second-level
// precision is not a goal.
const tickEvery = 30 * time.Second

// RunScheduler ticks until ctx ends. Call once, in a goroutine.
func (s *Service) RunScheduler(ctx context.Context) {
	t := time.NewTicker(tickEvery)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			s.tick()
		}
	}
}

// tick fires every due job. Exposed on the service (not the loop) so tests
// drive it directly with an injected clock.
func (s *Service) tick() {
	now := s.now()
	var due []Job
	s.mu.Lock()
	for i := range s.jobs {
		j := &s.jobs[i]
		at, ok := s.nextAt[j.ID]
		if !j.Enabled || !ok || now.Before(at) {
			continue
		}
		// Advance the schedule BEFORE running so a slow run can't double-fire,
		// and a "once" job disables itself at fire time (a failed run must not
		// refire forever).
		if j.Trigger.Kind == "once" {
			j.Enabled = false
			delete(s.nextAt, j.ID)
			s.saveLocked()
		} else {
			s.rescheduleLocked(j)
		}
		if s.running[j.ID] {
			s.recordRunLocked(Run{
				JobID:      j.ID,
				StartedAt:  now.UnixMilli(),
				FinishedAt: now.UnixMilli(),
				Status:     "skipped",
				Detail:     "previous run still in progress",
			})
			continue
		}
		s.running[j.ID] = true
		due = append(due, *j)
	}
	s.mu.Unlock()
	for _, j := range due {
		go s.execute(j)
	}
}

// execute runs one job's action and records the run. The caller must have set
// running[j.ID]; execute clears it.
func (s *Service) execute(j Job) {
	start := s.now()
	ctx, cancel := context.WithTimeout(context.Background(), runTimeout)
	defer cancel()

	detail, err := s.perform(ctx, j)
	run := Run{
		JobID:      j.ID,
		StartedAt:  start.UnixMilli(),
		FinishedAt: s.now().UnixMilli(),
		Status:     "ok",
		Detail:     truncate(detail, detailCap),
	}
	if skip := (*skipError)(nil); errors.As(err, &skip) {
		// A guard decided there was nothing worth waking a model for. That is
		// the job working, not failing: same `skipped` status an overlapping
		// fire records, and deliberately NO notify.post — a guard that fires
		// nightly must not notify nightly, or it trains the user to ignore it.
		run.Status = "skipped"
		run.Detail = truncate(err.Error(), detailCap)
		s.mu.Lock()
		delete(s.running, j.ID)
		s.recordRunLocked(run)
		s.mu.Unlock()
		return
	}
	if err != nil {
		run.Status = "error"
		run.Detail = truncate(err.Error(), detailCap)
		log.Printf("[jobs] %q failed: %v", j.Name, err)
		// Failures surface through the EXISTING notify.post event (desktop
		// notification center + web clients ingest it) — no new job.* topic,
		// which would cost four pinned registries.
		if s.b != nil {
			s.b.Publish(event.New("notify.post", "jobs", map[string]any{
				"title": fmt.Sprintf("Job failed: %s", j.Name),
				"body":  run.Detail,
				"level": "error",
				"key":   "job-" + j.ID,
			}))
		}
	}

	s.mu.Lock()
	delete(s.running, j.ID)
	s.recordRunLocked(run)
	s.mu.Unlock()
}

func (s *Service) recordRunLocked(r Run) {
	runs := append([]Run{r}, s.history[r.JobID]...)
	if len(runs) > maxRunsPerJob {
		runs = runs[:maxRunsPerJob]
	}
	s.history[r.JobID] = runs
	s.saveHistoryLocked()
}

// perform dispatches one action through the Runner.
func (s *Service) perform(ctx context.Context, j Job) (string, error) {
	switch j.Action.Kind {
	case "spawn":
		a := j.Action.Spawn
		// Context BEFORE spawn, always: the steps gather the prompt's material
		// and hold the veto, so a guard that says "nothing to do" costs a shell
		// command instead of an agent, a model call and a session to clean up.
		prompt, err := s.buildPrompt(ctx, a)
		if err != nil {
			return "", err
		}
		params := map[string]any{
			"cwd":   a.Cwd,
			"label": j.Name,
		}
		if a.Provider != "" {
			params["provider"] = a.Provider
		}
		if a.Model != "" {
			params["model"] = a.Model
		}
		if a.Effort != "" {
			params["effort"] = a.Effort
		}
		if a.PermissionMode != "" {
			params["permissionMode"] = a.PermissionMode
		}
		res, err := s.runner.Call(ctx, "agents.spawn", params)
		if err != nil {
			return "", fmt.Errorf("agents.spawn: %w", err)
		}
		var spawned struct {
			SessionID string `json:"sessionId"`
		}
		if err := json.Unmarshal(res, &spawned); err != nil || spawned.SessionID == "" {
			return "", fmt.Errorf("agents.spawn returned no sessionId: %s", truncate(string(res), 200))
		}
		// claudemon buffers a message sent before the session settles, so the
		// prompt can follow the spawn immediately.
		if _, err := s.runner.Call(ctx, "agents.sendMessage", map[string]any{
			"sessionId": spawned.SessionID,
			"text":      prompt,
		}); err != nil {
			return "", fmt.Errorf("spawned %s but prompt failed: %w", spawned.SessionID, err)
		}
		return "spawned " + spawned.SessionID, nil
	case "call":
		a := j.Action.Call
		var params any
		if len(a.Params) > 0 {
			params = json.RawMessage(a.Params)
		} else {
			params = map[string]any{}
		}
		res, err := s.runner.Call(ctx, a.Method, params)
		if err != nil {
			return "", fmt.Errorf("%s: %w", a.Method, err)
		}
		return string(res), nil
	case "shell":
		a := j.Action.Shell
		out, err := s.runner.Shell(ctx, a.Command, a.Cwd)
		if err != nil {
			return "", fmt.Errorf("%w — output: %s", err, tail(out, detailCap/2))
		}
		return tail(out, detailCap), nil
	default:
		return "", fmt.Errorf("unknown action kind %q", j.Action.Kind)
	}
}

// skipError ends a run early WITHOUT it counting as a failure — a context
// step's guard deciding the model isn't needed this time.
type skipError struct{ reason string }

func (e *skipError) Error() string { return e.reason }

// buildPrompt runs the spawn action's context steps and returns the prompt to
// send. A *skipError means: don't spawn, record `skipped`.
func (s *Service) buildPrompt(ctx context.Context, a *SpawnAction) (string, error) {
	if len(a.Context) == 0 {
		return a.Prompt, nil
	}
	outs := make([]string, 0, len(a.Context))
	for i, st := range a.Context {
		out, err := s.runContextStep(ctx, st)
		if err != nil {
			return "", fmt.Errorf("context step %d: %w", i+1, err)
		}
		outs = append(outs, out)
	}
	return fillPrompt(a.Prompt, outs), nil
}

// runContextStep produces one step's text, or a *skipError when its guard says
// the run should stop here.
func (s *Service) runContextStep(ctx context.Context, st ContextStep) (string, error) {
	var out string
	switch st.Kind {
	case "shell":
		raw, err := s.runner.Shell(ctx, st.Shell.Command, st.Shell.Cwd)
		if err != nil && !(st.IgnoreExitCode && isExitCodeErr(err)) {
			return "", fmt.Errorf("%w — output: %s", err, tail(raw, detailCap/2))
		}
		out = raw
	case "call":
		var params any = map[string]any{}
		if len(st.Call.Params) > 0 {
			params = json.RawMessage(st.Call.Params)
		}
		res, err := s.runner.Call(ctx, st.Call.Method, params)
		if err != nil {
			return "", fmt.Errorf("%s: %w", st.Call.Method, err)
		}
		out = string(res)
	default:
		return "", fmt.Errorf("unknown context step kind %q", st.Kind)
	}
	out = strings.TrimSpace(out)
	if st.SkipIfEmpty && isEmptyOutput(out) {
		return "", &skipError{reason: "no output — nothing to send an agent"}
	}
	if st.SkipUnlessMatch != "" {
		// Validate compiled this at save time; a pattern that somehow doesn't
		// compile here fails the run rather than silently spawning.
		re, err := regexp.Compile(st.SkipUnlessMatch)
		if err != nil {
			return "", fmt.Errorf("invalid skipUnlessMatch: %w", err)
		}
		if !re.MatchString(out) {
			return "", &skipError{reason: "output did not match " + st.SkipUnlessMatch}
		}
	}
	return elide(out, contextCap), nil
}

// isExitCodeErr is true only for "the command ran and exited nonzero" — what
// IgnoreExitCode forgives. A timeout or an unstartable command is a real
// failure and stays one, so a guard can't turn a broken job into a silent one.
func isExitCodeErr(err error) bool {
	var ee *exec.ExitError
	return errors.As(err, &ee)
}

// isEmptyOutput treats JSON's several spellings of nothing as empty, so
// SkipIfEmpty works on a `call` step the way it reads.
func isEmptyOutput(out string) bool {
	switch strings.TrimSpace(out) {
	case "", "{}", "[]", "null", `""`:
		return true
	}
	return false
}

// fillPrompt substitutes step outputs into the prompt: `{{output.N}}` (1-based)
// names a step, `{{output}}` the last one. A prompt that names none of them
// still gets the material — appended as fenced blocks rather than dropped,
// because a context step the model never sees is the confusing failure.
func fillPrompt(prompt string, outs []string) string {
	filled := prompt
	used := false
	for i, out := range outs {
		ph := fmt.Sprintf("{{output.%d}}", i+1)
		if strings.Contains(filled, ph) {
			filled = strings.ReplaceAll(filled, ph, out)
			used = true
		}
	}
	if len(outs) > 0 && strings.Contains(filled, "{{output}}") {
		filled = strings.ReplaceAll(filled, "{{output}}", outs[len(outs)-1])
		used = true
	}
	if used {
		return filled
	}
	var b strings.Builder
	b.WriteString(filled)
	for i, out := range outs {
		fmt.Fprintf(&b, "\n\n--- context step %d ---\n```\n%s\n```", i+1, out)
	}
	return b.String()
}

// elide keeps both ends of an oversized output and says how much it dropped —
// a head-only cut loses the failure summary, a tail-only cut loses what ran.
func elide(s string, n int) string {
	if len(s) <= n {
		return s
	}
	half := n / 2
	return fmt.Sprintf("%s\n\n… %d characters elided …\n\n%s", s[:half], len(s)-2*half, s[len(s)-half:])
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "…"
}

// tail keeps the END of shell output — the error is almost always there.
func tail(s string, n int) string {
	s = strings.TrimSpace(s)
	if len(s) <= n {
		return s
	}
	return "…" + s[len(s)-n:]
}

// BusRunner is the production Runner: capability calls go through the hub's
// self-dialed bus client (so provider clamps apply), shell commands through
// the host shell.
type BusRunner struct {
	// Call is wired to a busclient.Client's Call (kept as a func so cmd/hub
	// owns the client lifecycle and tests never need a socket).
	CallFn func(ctx context.Context, method string, params any) (json.RawMessage, error)
}

func (r *BusRunner) Call(ctx context.Context, method string, params any) (json.RawMessage, error) {
	return r.CallFn(ctx, method, params)
}

func (r *BusRunner) Shell(ctx context.Context, command, cwd string) (string, error) {
	var cmd *exec.Cmd
	if runtime.GOOS == "windows" {
		cmd = exec.CommandContext(ctx, "cmd", "/C", command)
	} else {
		cmd = exec.CommandContext(ctx, "/bin/sh", "-c", command)
	}
	if cwd != "" {
		cmd.Dir = cwd
	}
	out, err := cmd.CombinedOutput()
	return string(out), err
}
