package main

import (
	"context"
	"encoding/base64"
	"fmt"
	"os"
	"os/exec"
	"sync"

	"github.com/creack/pty"
	"github.com/google/uuid"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// terminalSession holds the state for a single PTY session.
type terminalSession struct {
	ptyFile *os.File
	cmd     *exec.Cmd
	cols    int
	rows    int
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
		if sess.cmd.Process != nil {
			_ = sess.cmd.Process.Kill()
		}
		_ = sess.ptyFile.Close()
		delete(ts.sessions, id)
	}
	return nil
}

// CreateTerminal creates a new PTY session running the given shell.
// If shell is empty, the default shell is detected from the environment.
// Returns the session ID.
func (ts *TerminalService) CreateTerminal(shell string) (string, error) {
	if shell == "" {
		shell = os.Getenv("SHELL")
		if shell == "" {
			shell = "/bin/sh"
		}
	}

	id := uuid.New().String()

	cmd := exec.Command(shell)
	cmd.Env = append(os.Environ(), "TERM=xterm-256color")

	cols := 80
	rows := 24

	ptmx, err := pty.StartWithSize(cmd, &pty.Winsize{
		Cols: uint16(cols),
		Rows: uint16(rows),
	})
	if err != nil {
		return "", fmt.Errorf("failed to start pty: %w", err)
	}

	sess := &terminalSession{
		ptyFile: ptmx,
		cmd:     cmd,
		cols:    cols,
		rows:    rows,
	}

	ts.mu.Lock()
	ts.sessions[id] = sess
	ts.mu.Unlock()

	// Start goroutine to read PTY output and emit events to the frontend.
	go ts.readPtyOutput(id, ptmx)

	return id, nil
}

// WriteTerminal writes base64-encoded input data to the PTY session.
func (ts *TerminalService) WriteTerminal(id string, data string) error {
	ts.mu.Lock()
	sess, ok := ts.sessions[id]
	ts.mu.Unlock()

	if !ok {
		return fmt.Errorf("terminal session not found: %s", id)
	}

	decoded, err := base64.StdEncoding.DecodeString(data)
	if err != nil {
		return fmt.Errorf("failed to decode base64 input: %w", err)
	}

	_, err = sess.ptyFile.Write(decoded)
	if err != nil {
		return fmt.Errorf("failed to write to pty: %w", err)
	}

	return nil
}

// ResizeTerminal resizes the PTY to the given columns and rows.
func (ts *TerminalService) ResizeTerminal(id string, cols int, rows int) error {
	ts.mu.Lock()
	sess, ok := ts.sessions[id]
	ts.mu.Unlock()

	if !ok {
		return fmt.Errorf("terminal session not found: %s", id)
	}

	err := pty.Setsize(sess.ptyFile, &pty.Winsize{
		Cols: uint16(cols),
		Rows: uint16(rows),
	})
	if err != nil {
		return fmt.Errorf("failed to resize pty: %w", err)
	}

	ts.mu.Lock()
	sess.cols = cols
	sess.rows = rows
	ts.mu.Unlock()

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
	delete(ts.sessions, id)
	ts.mu.Unlock()

	if sess.cmd.Process != nil {
		_ = sess.cmd.Process.Kill()
	}
	_ = sess.ptyFile.Close()

	// Wait for the process to finish to avoid zombies.
	_ = sess.cmd.Wait()

	app := application.Get()
	if app != nil {
		app.Event.Emit("terminal:"+id+":exit")
	}

	return nil
}

// readPtyOutput reads from the PTY in a loop and emits base64-encoded output events.
func (ts *TerminalService) readPtyOutput(id string, ptmx *os.File) {
	buf := make([]byte, 4096)

	for {
		n, err := ptmx.Read(buf)
		if n > 0 {
			encoded := base64.StdEncoding.EncodeToString(buf[:n])
			app := application.Get()
			if app != nil {
				app.Event.Emit("terminal:"+id+":output", encoded)
			}
		}
		if err != nil {
			// PTY closed or process exited.
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
