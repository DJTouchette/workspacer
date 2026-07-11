package main

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"sync"
	"sync/atomic"
	"syscall"
	"time"
)

// childSpec describes one supervised daemon.
type childSpec struct {
	Name string
	Bin  string
	Args []string
	Env  []string // extra env appended after os.Environ()
}

// child runs one daemon with the desktop's supervision semantics: restart on
// crash with exponential backoff, give up after repeated consecutive failures,
// SIGTERM-then-SIGKILL on stop. It exists (rather than reusing
// internal/supervisor) for two launcher-specific reasons: child stdout/stderr
// must reach the operator's terminal, prefixed — a headless server whose
// daemon logs vanish is undebuggable (internal/supervisor discards them) —
// and a persistently-crashing binary should give up loudly like the desktop
// does, not retry forever.
type child struct {
	spec childSpec
	logw io.Writer
	bo   *restartBackoff

	cancel context.CancelFunc
	done   chan struct{}
	gaveUp atomic.Bool

	// Parent-death pipe (see internal/parentwatch): the child gets parentR as
	// stdin and we hold parentW for our whole life. If this launcher is
	// force-killed (SIGKILL — no chance to SIGTERM anyone), the OS closes the
	// pipe, the child sees stdin EOF, and self-exits instead of orphaning and
	// holding its ports. The struct reference keeps parentW alive.
	parentR, parentW *os.File
}

// startChild launches the supervision loop in the background. The backoff is
// passed in (rather than constructed here) so tests can shrink its delays and
// budget without sleeping through real seconds.
func startChild(parent context.Context, spec childSpec, logw io.Writer, bo *restartBackoff) *child {
	ctx, cancel := context.WithCancel(parent)
	c := &child{
		spec:   spec,
		logw:   logw,
		bo:     bo,
		cancel: cancel,
		done:   make(chan struct{}),
	}
	// Best-effort: without the pipe, graceful stop still works via SIGTERM.
	if r, w, err := os.Pipe(); err == nil {
		c.parentR, c.parentW = r, w
	}
	go func() {
		c.run(ctx)
		close(c.done)
	}()
	return c
}

// Stop asks the child to terminate and waits for the loop to finish.
func (c *child) Stop() {
	c.cancel()
	<-c.done
}

// GaveUp reports whether supervision hit the restart budget and stopped.
func (c *child) GaveUp() bool { return c.gaveUp.Load() }

func (c *child) run(ctx context.Context) {
	// WORKSPACER_PARENT_PID both documents who launched the daemon and gates
	// its stdin-EOF/parent-poll watchdog (a manual `hub` run must never
	// self-exit on a closed stdin).
	env := append(append([]string{}, c.spec.Env...),
		"WORKSPACER_PARENT_PID="+strconv.Itoa(os.Getpid()))

	for ctx.Err() == nil {
		cmd := exec.CommandContext(ctx, c.spec.Bin, c.spec.Args...)
		cmd.Env = append(os.Environ(), env...)
		if c.parentR != nil {
			cmd.Stdin = c.parentR
		}
		prefix := "[" + c.spec.Name + "] "
		outw := &prefixWriter{w: c.logw, prefix: prefix}
		errw := &prefixWriter{w: c.logw, prefix: prefix}
		cmd.Stdout = outw
		cmd.Stderr = errw
		// Graceful stop: SIGTERM on cancel (plain Kill on Windows, where
		// Signal(SIGTERM) is unsupported — the hub's job object reaps its tree
		// there), escalating to SIGKILL if it lingers past WaitDelay.
		cmd.Cancel = func() error { return terminate(cmd.Process) }
		cmd.WaitDelay = 6 * time.Second

		c.bo.markStarted()
		err := cmd.Start()
		if err == nil {
			fmt.Fprintf(c.logw, "[workspacer] %s started (pid %d)\n", c.spec.Name, cmd.Process.Pid)
			err = cmd.Wait()
			outw.flush()
			errw.flush()
		}
		if ctx.Err() != nil { // we asked it to stop — not a crash
			return
		}

		delay, ok := c.bo.nextDelay()
		if !ok {
			c.gaveUp.Store(true)
			fmt.Fprintf(c.logw, "[workspacer] %s keeps crashing — gave up restarting it (last error: %v)\n",
				c.spec.Name, err)
			return
		}
		fmt.Fprintf(c.logw, "[workspacer] %s exited unexpectedly (%v) — restarting in %s\n",
			c.spec.Name, err, delay)
		select {
		case <-ctx.Done():
			return
		case <-time.After(delay):
		}
	}
}

// terminate sends the polite stop signal for the platform.
func terminate(p *os.Process) error {
	if p == nil {
		return nil
	}
	if runtime.GOOS == "windows" {
		return p.Kill()
	}
	return p.Signal(syscall.SIGTERM)
}

// prefixWriter prefixes every line of a child's output with its name so the
// interleaved claudemon/hub/brain logs stay attributable. Partial writes are
// buffered until their newline arrives; the supervisor flushes any trailing
// unterminated line after the process exits.
type prefixWriter struct {
	mu     sync.Mutex
	w      io.Writer
	prefix string
	buf    []byte
}

func (p *prefixWriter) Write(b []byte) (int, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.buf = append(p.buf, b...)
	for {
		i := bytes.IndexByte(p.buf, '\n')
		if i < 0 {
			break
		}
		if _, err := io.WriteString(p.w, p.prefix+string(p.buf[:i+1])); err != nil {
			return len(b), err
		}
		p.buf = p.buf[i+1:]
	}
	return len(b), nil
}

// flush writes any buffered final line (a child that died mid-line still gets
// its last words logged).
func (p *prefixWriter) flush() {
	p.mu.Lock()
	defer p.mu.Unlock()
	if len(p.buf) > 0 {
		_, _ = io.WriteString(p.w, p.prefix+string(p.buf)+"\n")
		p.buf = nil
	}
}
