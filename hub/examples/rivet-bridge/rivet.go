package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"os/exec"
	"sync"
	"sync/atomic"
	"time"
)

// rivetClient owns a long-lived `rivet serve` subprocess and speaks MCP
// (JSON-RPC 2.0, line-delimited) to it over stdin/stdout. Rivet has no HTTP/SSE
// transport — stdio is the only option — so we supervise the process ourselves
// and reconnect with backoff if it dies, mirroring the bus client's loop.
//
// Calls are correlated by JSON-RPC id: a single reader goroutine drains stdout
// and hands each response to the channel registered under its id.
type rivetClient struct {
	bin        string // path to the rivet binary
	projectDir string // cwd for `rivet serve` (must contain/anchor .rivet/)
	debug      bool

	mu    sync.Mutex // guards stdin + the live process
	stdin io.WriteCloser

	pendMu  sync.Mutex
	pending map[int]chan rpcResponse
	seq     int

	ready atomic.Bool

	// lifecycle callbacks (set by main); fired on first init and on each exit.
	onReady func(initResult json.RawMessage)
	onDown  func(err error)
}

func newRivetClient(bin, projectDir string, debug bool) *rivetClient {
	return &rivetClient{
		bin:        bin,
		projectDir: projectDir,
		debug:      debug,
		pending:    map[int]chan rpcResponse{},
	}
}

// --- JSON-RPC 2.0 wire types ---

type rpcRequest struct {
	JSONRPC string `json:"jsonrpc"`
	ID      *int   `json:"id,omitempty"` // nil => notification
	Method  string `json:"method"`
	Params  any    `json:"params,omitempty"`
}

type rpcResponse struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      *int            `json:"id"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   *rpcError       `json:"error,omitempty"`
}

type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

// toolResult is the MCP tools/call result shape (content blocks + error flag).
type toolResult struct {
	Content []struct {
		Type string `json:"type"`
		Text string `json:"text"`
	} `json:"content"`
	IsError bool `json:"isError"`
}

// run supervises the subprocess: (re)start, handshake, serve, and on exit wait
// with backoff before trying again. Blocks until ctx is cancelled.
func (r *rivetClient) run(ctx context.Context) {
	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		err := r.session(ctx)
		r.ready.Store(false)
		if r.onDown != nil {
			r.onDown(err)
		}
		if err != nil && ctx.Err() == nil {
			log.Printf("rivet-bridge: rivet serve exited (%v); restarting in %s", err, backoff)
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(backoff):
		}
		if backoff < 10*time.Second {
			backoff *= 2
		}
	}
}

// session runs one process lifetime: spawn, drain stdout, initialize, then block
// until the process exits or ctx is cancelled.
func (r *rivetClient) session(ctx context.Context) error {
	cmd := exec.CommandContext(ctx, r.bin, "serve")
	cmd.Dir = r.projectDir
	if r.debug {
		cmd.Args = append(cmd.Args, "--debug")
	}

	stdin, err := cmd.StdinPipe()
	if err != nil {
		return fmt.Errorf("stdin pipe: %w", err)
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return fmt.Errorf("stdout pipe: %w", err)
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return fmt.Errorf("stderr pipe: %w", err)
	}

	if err := cmd.Start(); err != nil {
		return fmt.Errorf("start %s serve (dir %s): %w", r.bin, r.projectDir, err)
	}
	log.Printf("rivet-bridge: started %s serve (pid %d) in %s", r.bin, cmd.Process.Pid, r.projectDir)

	r.mu.Lock()
	r.stdin = stdin
	r.mu.Unlock()

	// Surface rivet's stderr (it logs there) for debugging.
	go r.drainStderr(stderr)

	// Reader goroutine: dispatch responses to waiting callers by id. It returns
	// when stdout closes (process exit), which unblocks the read loop below.
	readDone := make(chan struct{})
	go func() {
		r.readLoop(stdout)
		close(readDone)
	}()

	defer func() {
		r.mu.Lock()
		r.stdin = nil
		r.mu.Unlock()
		_ = stdin.Close()
		r.failPending(errors.New("rivet serve disconnected"))
	}()

	// MCP handshake. If it fails, tear the session down and let run() retry.
	if err := r.initialize(ctx); err != nil {
		_ = cmd.Process.Kill()
		<-readDone
		_ = cmd.Wait()
		return fmt.Errorf("initialize: %w", err)
	}

	// Block until the process exits (readDone) or we're asked to stop.
	select {
	case <-ctx.Done():
		_ = cmd.Process.Kill()
		<-readDone
		_ = cmd.Wait()
		return ctx.Err()
	case <-readDone:
		return cmd.Wait()
	}
}

// initialize performs the MCP initialize handshake and marks the client ready.
func (r *rivetClient) initialize(ctx context.Context) error {
	params := map[string]any{
		"protocolVersion": "2024-11-05",
		"capabilities":    map[string]any{},
		"clientInfo":      map[string]any{"name": "rivet-bridge", "version": "0.1.0"},
	}
	ictx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	res, err := r.rpc(ictx, "initialize", params)
	if err != nil {
		return err
	}
	// Best-effort notify; rivet's hand-rolled server ignores unknown notifications.
	_ = r.notify("notifications/initialized", nil)

	r.ready.Store(true)
	log.Printf("rivet-bridge: MCP session initialized")
	if r.onReady != nil {
		r.onReady(res)
	}
	return nil
}

// callTool invokes an MCP tool and returns its flattened text output. name is a
// rivet tool name (e.g. "recon.symbols"); args is the raw JSON arguments object
// forwarded from the bus caller (e.g. {"args":["foo"]}), or nil for none.
func (r *rivetClient) callTool(ctx context.Context, name string, args json.RawMessage) (json.RawMessage, error) {
	if !r.ready.Load() {
		return nil, errors.New("rivet not ready (serve not connected)")
	}
	arguments := json.RawMessage(args)
	if len(arguments) == 0 {
		arguments = json.RawMessage(`{}`)
	}
	params := map[string]json.RawMessage{
		"name":      mustJSON(name),
		"arguments": arguments,
	}
	cctx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	res, err := r.rpc(cctx, "tools/call", params)
	if err != nil {
		return nil, err
	}

	var tr toolResult
	if err := json.Unmarshal(res, &tr); err != nil {
		// Not the shape we expected — pass the raw result straight through.
		return res, nil
	}
	var text string
	for i, c := range tr.Content {
		if i > 0 {
			text += "\n"
		}
		text += c.Text
	}
	out := map[string]any{"tool": name, "output": text, "isError": tr.IsError}
	return mustJSON(out), nil
}

// rpc sends a request and waits for its response (or ctx timeout).
func (r *rivetClient) rpc(ctx context.Context, method string, params any) (json.RawMessage, error) {
	r.pendMu.Lock()
	r.seq++
	id := r.seq
	ch := make(chan rpcResponse, 1)
	r.pending[id] = ch
	r.pendMu.Unlock()

	if err := r.send(rpcRequest{JSONRPC: "2.0", ID: &id, Method: method, Params: params}); err != nil {
		r.pendMu.Lock()
		delete(r.pending, id)
		r.pendMu.Unlock()
		return nil, err
	}

	select {
	case resp := <-ch:
		if resp.Error != nil {
			return nil, fmt.Errorf("rivet rpc error %d: %s", resp.Error.Code, resp.Error.Message)
		}
		return resp.Result, nil
	case <-ctx.Done():
		r.pendMu.Lock()
		delete(r.pending, id)
		r.pendMu.Unlock()
		return nil, ctx.Err()
	}
}

func (r *rivetClient) notify(method string, params any) error {
	return r.send(rpcRequest{JSONRPC: "2.0", Method: method, Params: params})
}

// send writes one newline-delimited JSON-RPC message to rivet's stdin.
func (r *rivetClient) send(req rpcRequest) error {
	data, err := json.Marshal(req)
	if err != nil {
		return err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.stdin == nil {
		return errors.New("rivet stdin closed")
	}
	data = append(data, '\n')
	_, err = r.stdin.Write(data)
	return err
}

// readLoop drains stdout, parsing one JSON-RPC response per line and handing it
// to the caller waiting on its id. Server-initiated notifications/requests
// (no matching pending id) are ignored.
func (r *rivetClient) readLoop(stdout io.Reader) {
	br := bufio.NewReaderSize(stdout, 1<<20)
	for {
		line, err := br.ReadBytes('\n')
		if len(line) > 0 {
			var resp rpcResponse
			if jerr := json.Unmarshal(line, &resp); jerr == nil && resp.ID != nil {
				r.pendMu.Lock()
				ch, ok := r.pending[*resp.ID]
				if ok {
					delete(r.pending, *resp.ID)
				}
				r.pendMu.Unlock()
				if ok {
					ch <- resp
				}
			}
		}
		if err != nil {
			return
		}
	}
}

func (r *rivetClient) drainStderr(stderr io.Reader) {
	sc := bufio.NewScanner(stderr)
	sc.Buffer(make([]byte, 0, 64*1024), 1<<20)
	for sc.Scan() {
		if r.debug {
			log.Printf("rivet: %s", sc.Text())
		}
	}
}

func (r *rivetClient) failPending(err error) {
	r.pendMu.Lock()
	for id, ch := range r.pending {
		ch <- rpcResponse{ID: &id, Error: &rpcError{Code: -1, Message: err.Error()}}
		delete(r.pending, id)
	}
	r.pendMu.Unlock()
}

func (r *rivetClient) isReady() bool { return r.ready.Load() }

func mustJSON(v any) json.RawMessage {
	data, err := json.Marshal(v)
	if err != nil {
		return json.RawMessage(`null`)
	}
	return data
}
