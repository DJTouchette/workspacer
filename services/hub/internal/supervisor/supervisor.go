// Package supervisor runs a managed child process: spawn, health-check,
// restart-on-crash, graceful stop. It generalizes the bespoke per-daemon
// spawning the Electron app does today and reports lifecycle on the event bus.
package supervisor

import (
	"context"
	"net/http"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/event"
)

// Publisher is the slice of the broker the supervisor needs.
type Publisher interface {
	Publish(event.Envelope)
}

// State is the coarse lifecycle of the managed process.
type State string

const (
	Stopped   State = "stopped"
	Running   State = "running"
	Healthy   State = "healthy"
	Unhealthy State = "unhealthy"
	Crashed   State = "crashed"
)

// Spec describes a process to manage.
type Spec struct {
	Name    string
	Command string
	Args    []string
	Env     []string // extra env appended to os.Environ()
	Dir     string

	// HealthURL, if set, is polled with GET; 200 == healthy.
	HealthURL    string
	HealthPeriod time.Duration // default 2s
	RestartWait  time.Duration // backoff before restart, default 1s
}

func (s Spec) healthPeriod() time.Duration {
	if s.HealthPeriod > 0 {
		return s.HealthPeriod
	}
	return 2 * time.Second
}

func (s Spec) restartWait() time.Duration {
	if s.RestartWait > 0 {
		return s.RestartWait
	}
	return time.Second
}

// statusData is the payload of sidecar.* events.
type statusData struct {
	Name  string `json:"name"`
	State State  `json:"state"`
	PID   int    `json:"pid,omitempty"`
	Err   string `json:"err,omitempty"`
}

// Supervisor manages one process per Spec.
type Supervisor struct {
	spec   Spec
	pub    Publisher
	client *http.Client

	mu      sync.Mutex
	state   State
	cancel  context.CancelFunc
	done    chan struct{}
	healthy atomic.Bool

	// Parent-death pipe. Each child gets parentR as stdin; we hold parentW open
	// for this process's lifetime. If the hub is force-killed, the OS closes
	// parentW and the child sees stdin EOF and self-exits (see parentwatch). A
	// field reference keeps parentW from being closed by os.File's GC finalizer.
	parentR *os.File
	parentW *os.File
}

// New creates a supervisor. pub may be nil (no lifecycle events emitted).
func New(spec Spec, pub Publisher) *Supervisor {
	return &Supervisor{
		spec:   spec,
		pub:    pub,
		client: &http.Client{Timeout: 2 * time.Second},
		state:  Stopped,
	}
}

// Start launches the manage loop in the background. Idempotent: if the
// supervisor is already running, Start is a no-op. Call Stop before starting
// again with different settings.
func (s *Supervisor) Start() {
	s.mu.Lock()
	if s.done != nil {
		select {
		case <-s.done:
			// Previous run has finished; allow a fresh start below.
		default:
			// Still running — do nothing.
			s.mu.Unlock()
			return
		}
	}
	ctx, cancel := context.WithCancel(context.Background())
	s.cancel = cancel
	s.done = make(chan struct{})
	done := s.done
	s.mu.Unlock()
	go func() {
		s.run(ctx)
		close(done)
	}()
}

// Stop signals the process to terminate and waits for the loop to exit.
func (s *Supervisor) Stop() {
	s.mu.Lock()
	cancel := s.cancel
	done := s.done
	s.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	if done != nil {
		<-done
	}
}

// State returns the current lifecycle state.
func (s *Supervisor) State() State {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.state
}

func (s *Supervisor) setState(st State) {
	s.mu.Lock()
	s.state = st
	s.mu.Unlock()
}

func (s *Supervisor) emit(st State, pid int, errMsg string) {
	if s.pub == nil {
		return
	}
	s.pub.Publish(event.New("sidecar."+string(st), "supervisor", statusData{
		Name: s.spec.Name, State: st, PID: pid, Err: errMsg,
	}))
}

func (s *Supervisor) run(ctx context.Context) {
	// Open the parent-death pipe once; reused across restarts. Best-effort — if
	// it fails we just spawn without it (graceful stop still works via Cancel).
	if s.parentR == nil {
		if r, w, err := os.Pipe(); err == nil {
			s.parentR, s.parentW = r, w
		}
	}
	// Tell children who their parent is; this both documents the relationship
	// and gates each child's stdin-EOF watchdog (see internal/parentwatch).
	childEnv := append(append([]string{}, s.spec.Env...),
		"WORKSPACER_PARENT_PID="+strconv.Itoa(os.Getpid()))

	for {
		if ctx.Err() != nil {
			s.setState(Stopped)
			return
		}

		cmd := exec.CommandContext(ctx, s.spec.Command, s.spec.Args...)
		cmd.Env = mergeEnv(os.Environ(), childEnv)
		cmd.Dir = s.spec.Dir
		if s.parentR != nil {
			cmd.Stdin = s.parentR
		}
		// Graceful stop: SIGTERM on cancel, SIGKILL if it lingers.
		cmd.Cancel = func() error { return cmd.Process.Signal(syscall.SIGTERM) }
		cmd.WaitDelay = 5 * time.Second

		if err := cmd.Start(); err != nil {
			s.setState(Crashed)
			s.emit(Crashed, 0, err.Error())
			if !sleepOrDone(ctx, s.spec.restartWait()) {
				s.setState(Stopped)
				return
			}
			continue
		}

		s.setState(Running)
		s.emit(Running, cmd.Process.Pid, "")

		hctx, hcancel := context.WithCancel(ctx)
		if s.spec.HealthURL != "" {
			go s.healthLoop(hctx, cmd.Process.Pid)
		}

		err := cmd.Wait()
		hcancel()
		s.healthy.Store(false)

		if ctx.Err() != nil { // we asked it to stop
			s.setState(Stopped)
			s.emit(Stopped, 0, "")
			return
		}

		// Unexpected exit — report and restart after backoff.
		s.setState(Crashed)
		msg := ""
		if err != nil {
			msg = err.Error()
		}
		s.emit(Crashed, 0, msg)
		if !sleepOrDone(ctx, s.spec.restartWait()) {
			s.setState(Stopped)
			return
		}
	}
}

func (s *Supervisor) healthLoop(ctx context.Context, pid int) {
	t := time.NewTicker(s.spec.healthPeriod())
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			ok := s.ping(ctx)
			was := s.healthy.Swap(ok)
			switch {
			case ok && !was:
				s.setState(Healthy)
				s.emit(Healthy, pid, "")
			case !ok && was:
				s.setState(Unhealthy)
				s.emit(Unhealthy, pid, "")
			}
		}
	}
}

func (s *Supervisor) ping(ctx context.Context) bool {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, s.spec.HealthURL, nil)
	if err != nil {
		return false
	}
	resp, err := s.client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

// mergeEnv returns base with extra applied as overrides: an extra "KEY=val"
// replaces any same-KEY entry in base rather than appending a duplicate (whose
// resolution by the child is platform-dependent). New keys are appended.
func mergeEnv(base, extra []string) []string {
	if len(extra) == 0 {
		return base
	}
	out := append([]string{}, base...)
	idx := make(map[string]int, len(out))
	for i, kv := range out {
		if eq := strings.IndexByte(kv, '='); eq >= 0 {
			idx[kv[:eq]] = i
		}
	}
	for _, kv := range extra {
		eq := strings.IndexByte(kv, '=')
		if eq < 0 {
			out = append(out, kv)
			continue
		}
		if i, ok := idx[kv[:eq]]; ok {
			out[i] = kv
		} else {
			idx[kv[:eq]] = len(out)
			out = append(out, kv)
		}
	}
	return out
}

// sleepOrDone waits for d, returning false if ctx is cancelled first.
func sleepOrDone(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-t.C:
		return true
	}
}
