package main

import (
	"context"
	"encoding/base64"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"sync"

	gopty "github.com/aymanbagabas/go-pty"
	"github.com/google/uuid"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// terminalSession holds the state for a single PTY session.
type terminalSession struct {
	pty    gopty.Pty
	cmd    *gopty.Cmd
	cols   int
	rows   int
	closed bool
}

// TerminalService manages pseudo-terminal sessions for the frontend.
type TerminalService struct {
	mu       sync.Mutex
	sessions map[string]*terminalSession
}

// ServiceStartup initializes the TerminalService when the application starts.
func (ts *TerminalService) ServiceStartup(ctx context.Context, options application.ServiceOptions) error {
	ts.sessions = make(map[string]*terminalSession)
	return nil
}

// ServiceShutdown cleans up all remaining PTY sessions when the application shuts down.
func (ts *TerminalService) ServiceShutdown() error {
	ts.mu.Lock()
	defer ts.mu.Unlock()

	for id, sess := range ts.sessions {
		sess.closed = true
		if sess.cmd.Process != nil {
			_ = sess.cmd.Process.Kill()
		}
		_ = sess.pty.Close()
		delete(ts.sessions, id)
	}
	return nil
}

// defaultShell returns the default shell for the current platform.
func defaultShell() string {
	if runtime.GOOS == "windows" {
		if _, err := exec.LookPath("pwsh.exe"); err == nil {
			return "pwsh.exe"
		}
		return "powershell.exe"
	}
	if shell := os.Getenv("SHELL"); shell != "" {
		return shell
	}
	return "/bin/sh"
}

// CreateTerminal creates a new PTY session running the given shell.
// If shell is empty, the default shell is detected from the environment.
// Returns the session ID.
func (ts *TerminalService) CreateTerminal(shell string) (string, error) {
	if shell == "" {
		shell = defaultShell()
	}

	id := uuid.New().String()
	cols := 80
	rows := 24

	p, err := gopty.New()
	if err != nil {
		return "", fmt.Errorf("failed to create pty: %w", err)
	}

	if err := p.Resize(cols, rows); err != nil {
		_ = p.Close()
		return "", fmt.Errorf("failed to set initial size: %w", err)
	}

	cmd := p.Command(shell)
	cmd.Env = append(os.Environ(), "TERM=xterm-256color")

	if err := cmd.Start(); err != nil {
		_ = p.Close()
		return "", fmt.Errorf("failed to start shell: %w", err)
	}

	sess := &terminalSession{
		pty:  p,
		cmd:  cmd,
		cols: cols,
		rows: rows,
	}

	ts.mu.Lock()
	ts.sessions[id] = sess
	ts.mu.Unlock()

	go ts.readPtyOutput(id, p)

	go func() {
		_ = cmd.Wait()
	}()

	return id, nil
}

// WriteTerminal writes base64-encoded input data to the PTY session.
func (ts *TerminalService) WriteTerminal(id string, data string) error {
	ts.mu.Lock()
	sess, ok := ts.sessions[id]
	if !ok || sess.closed {
		ts.mu.Unlock()
		return fmt.Errorf("terminal session not found: %s", id)
	}

	decoded, err := base64.StdEncoding.DecodeString(data)
	if err != nil {
		ts.mu.Unlock()
		return fmt.Errorf("failed to decode base64 input: %w", err)
	}

	_, err = sess.pty.Write(decoded)
	ts.mu.Unlock()

	if err != nil {
		return fmt.Errorf("failed to write to pty: %w", err)
	}
	return nil
}

// ResizeTerminal resizes the PTY to the given columns and rows.
func (ts *TerminalService) ResizeTerminal(id string, cols int, rows int) error {
	ts.mu.Lock()
	sess, ok := ts.sessions[id]
	if !ok || sess.closed {
		ts.mu.Unlock()
		return fmt.Errorf("terminal session not found: %s", id)
	}

	err := sess.pty.Resize(cols, rows)
	if err == nil {
		sess.cols = cols
		sess.rows = rows
	}
	ts.mu.Unlock()

	if err != nil {
		return fmt.Errorf("failed to resize pty: %w", err)
	}
	return nil
}

// CloseTerminal kills the process, closes the PTY, and removes the session.
func (ts *TerminalService) CloseTerminal(id string) error {
	ts.mu.Lock()
	sess, ok := ts.sessions[id]
	if !ok {
		ts.mu.Unlock()
		return fmt.Errorf("terminal session not found: %s", id)
	}
	sess.closed = true
	delete(ts.sessions, id)
	ts.mu.Unlock()

	if sess.cmd.Process != nil {
		_ = sess.cmd.Process.Kill()
	}
	_ = sess.pty.Close()

	// Don't emit exit here — readPtyOutput will emit it when the read loop ends.
	// This prevents duplicate exit events.

	return nil
}

// readPtyOutput reads from the PTY in a loop and emits base64-encoded output events.
func (ts *TerminalService) readPtyOutput(id string, p gopty.Pty) {
	buf := make([]byte, 4096)

	for {
		n, err := p.Read(buf)
		if n > 0 {
			encoded := base64.StdEncoding.EncodeToString(buf[:n])
			app := application.Get()
			if app != nil {
				app.Event.Emit("terminal:"+id+":output", encoded)
			}
		}
		if err != nil {
			// PTY closed or process exited — clean up and notify frontend.
			ts.mu.Lock()
			_, stillExists := ts.sessions[id]
			if stillExists {
				delete(ts.sessions, id)
			}
			ts.mu.Unlock()

			app := application.Get()
			if app != nil {
				app.Event.Emit("terminal:"+id+":exit")
			}
			return
		}
	}
}
