package main

// A brain-owned Node process executes the SAME pure desktop services used by
// Electron. It has no listener or pairing credential of its own: calls arrive
// through the hub's policy, and roots/live identities are added here, outside
// caller-controlled params. Node never runs in the hub's credential context.
import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/uploads"
)

type spawnTemplateCwdKey struct{}

type desktopReply struct {
	HostCallID string          `json:"hostCallId"`
	Method     string          `json:"method"`
	Params     json.RawMessage `json:"params"`
	ID         string          `json:"id"`
	Result     json.RawMessage `json:"result"`
	Error      string          `json:"error"`
	Event      string          `json:"event"`
	Data       json.RawMessage `json:"data"`
}
type desktopProcess struct {
	cmd     *exec.Cmd
	input   io.WriteCloser
	pending map[string]chan desktopReply
}
type desktopHost struct {
	mu         sync.Mutex
	process    *desktopProcess
	next       uint64
	bundle     string
	onEvent    func(string, json.RawMessage)
	onHostCall func(context.Context, string, json.RawMessage) (json.RawMessage, error)
}

func (h *desktopHost) bundlePath() string {
	if h.bundle != "" {
		return h.bundle
	}
	if value := os.Getenv("WKS_DESKTOP_HOST"); value != "" {
		return value
	}
	exe, err := os.Executable()
	if err != nil {
		return ""
	}
	return filepath.Join(filepath.Dir(exe), "desktop-host.cjs")
}
func (h *desktopHost) available() bool {
	info, err := os.Stat(h.bundlePath())
	return err == nil && info.Mode().IsRegular()
}

func (h *desktopHost) startLocked() (*desktopProcess, error) {
	if h.process != nil {
		return h.process, nil
	}
	bundle := h.bundlePath()
	if _, err := os.Stat(bundle); err != nil {
		return nil, fmt.Errorf("desktop services are not installed: build/install desktop-host.cjs beside brain")
	}
	cmd := exec.Command("node", bundle)
	cmd.Stderr = os.Stderr
	input, err := cmd.StdinPipe()
	if err != nil {
		return nil, err
	}
	output, err := cmd.StdoutPipe()
	if err != nil {
		input.Close()
		return nil, err
	}
	if err = cmd.Start(); err != nil {
		input.Close()
		output.Close()
		return nil, fmt.Errorf("start desktop services (Node.js required): %w", err)
	}
	process := &desktopProcess{cmd: cmd, input: input, pending: map[string]chan desktopReply{}}
	h.process = process
	go func() {
		scanner := bufio.NewScanner(output)
		scanner.Buffer(make([]byte, 64*1024), 32*1024*1024)
		for scanner.Scan() {
			var reply desktopReply
			if json.Unmarshal(scanner.Bytes(), &reply) != nil {
				continue
			}
			if reply.HostCallID != "" {
				go func(request desktopReply) {
					callbackCtx, cancel := context.WithTimeout(context.Background(), 55*time.Second)
					defer cancel()
					var result json.RawMessage
					var err error
					if h.onHostCall == nil {
						err = fmt.Errorf("lifecycle callback unavailable")
					} else {
						result, err = h.onHostCall(callbackCtx, request.Method, request.Params)
					}
					response := map[string]any{"hostResultId": request.HostCallID, "result": result}
					if err != nil {
						response["error"] = err.Error()
					}
					data, _ := json.Marshal(response)
					h.mu.Lock()
					defer h.mu.Unlock()
					if h.process == process {
						_, _ = process.input.Write(append(data, '\n'))
					}
				}(reply)
				continue
			}
			if reply.Event != "" {
				if h.onEvent != nil {
					h.onEvent(reply.Event, reply.Data)
				}
				continue
			}
			if reply.ID == "" {
				continue
			}
			h.mu.Lock()
			ch := process.pending[reply.ID]
			delete(process.pending, reply.ID)
			h.mu.Unlock()
			if ch != nil {
				ch <- reply
			}
		}
		_ = cmd.Wait()
		h.mu.Lock()
		defer h.mu.Unlock()
		if h.process == process {
			h.process = nil
		}
		for id, ch := range process.pending {
			ch <- desktopReply{ID: id, Error: "Desktop service exited before acknowledging the operation; its outcome is unknown"}
			delete(process.pending, id)
		}
	}()
	return process, nil
}

func (h *desktopHost) call(ctx context.Context, method string, params json.RawMessage, hostContext any) (json.RawMessage, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	h.mu.Lock()
	process, err := h.startLocked()
	if err != nil {
		h.mu.Unlock()
		return nil, err
	}
	h.next++
	id := strconv.FormatUint(h.next, 10)
	ch := make(chan desktopReply, 1)
	process.pending[id] = ch
	if len(params) == 0 {
		params = json.RawMessage(`{}`)
	}
	message, err := json.Marshal(map[string]any{"id": id, "method": method, "params": params, "context": hostContext})
	if err == nil {
		_, err = process.input.Write(append(message, '\n'))
	}
	if err != nil {
		delete(process.pending, id)
		h.mu.Unlock()
		return nil, err
	}
	h.mu.Unlock()
	select {
	case reply := <-ch:
		if reply.Error != "" {
			return nil, errors.New(reply.Error)
		}
		return reply.Result, nil
	case <-ctx.Done():
		h.mu.Lock()
		delete(process.pending, id)
		h.mu.Unlock()
		return nil, fmt.Errorf("desktop service acknowledgement interrupted; operation outcome may be unknown: %w", ctx.Err())
	}
}

func (h *desktopHost) close() {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.process != nil {
		_ = h.process.input.Close()
	}
}

func (r *registry) desktopCall(ctx context.Context, method string, params json.RawMessage) (json.RawMessage, error) {
	if !capspec.IsDesktopService(method) {
		return nil, fmt.Errorf("unknown desktop service %q", method)
	}
	if method == "desktop.sessionGrantReconcile" {
		var p struct {
			SessionID string `json:"sessionId"`
			Role      string `json:"role"`
		}
		if err := json.Unmarshal(params, &p); err != nil {
			return nil, err
		}
		if p.Role != "manager" || p.SessionID == "" {
			return jsonResult(false)
		}
		session, ok := findFleetSession(r.fleetSessions(ctx), p.SessionID)
		if !ok || session.ended() || !session.IsWakeTarget {
			return jsonResult(false)
		}
		changed, err := r.reconcileManagerGrants(p.SessionID)
		if err != nil {
			return nil, err
		}
		return jsonResult(changed)
	}
	if method == "desktop.managerRequestSend" {
		return r.sendCapturedRequest(ctx, params)
	}
	if method == "desktop.agentRuntimeStatus" {
		return r.desktopRuntimeStatus(ctx)
	}
	if method == "desktop.keepWarmHeartbeats" {
		var p struct {
			Limit int `json:"limit"`
		}
		if err := unmarshal(params, &p); err != nil {
			return nil, err
		}
		if p.Limit <= 0 {
			p.Limit = 20
		}
		if p.Limit > 200 {
			p.Limit = 200
		}
		return r.cm.getRaw(ctx, "/heartbeats?limit="+strconv.Itoa(p.Limit))
	}
	return r.desktopInternalCall(ctx, method, params)
}

func (r *registry) desktopEvent(event string, data json.RawMessage) {
	if event == "workflow.event" && r.publish != nil {
		var envelope struct {
			Type string          `json:"type"`
			Data json.RawMessage `json:"data"`
		}
		if json.Unmarshal(data, &envelope) != nil {
			return
		}
		switch envelope.Type {
		case "workflow.started":
			r.publish("workflow.started", envelope.Data)
		case "workflow.completed":
			r.publish("workflow.completed", envelope.Data)
		case "workflow.failed":
			r.publish("workflow.failed", envelope.Data)
		case "workflow.agent.finished":
			r.publish("workflow.agent.finished", envelope.Data)
		}
		return
	}
	if event != "workflow.update" || r.store == nil {
		return
	}
	var update struct {
		SessionID string `json:"sessionId"`
	}
	if json.Unmarshal(data, &update) != nil || update.SessionID == "" {
		return
	}
	r.store.setDesktopWorkflow(update.SessionID, data)
}

func (r *registry) attachDesktopResult(ctx context.Context, entry *fleetEntry, reply string) bool {
	if !r.desktopServices.available() {
		return true
	}
	var before json.RawMessage
	if r.store != nil {
		before, _ = r.store.get(entry.SessionID)
	}
	params, _ := json.Marshal(map[string]string{"sessionId": entry.SessionID, "reply": reply})
	raw, err := r.desktopInternalCall(ctx, "internal.finishWorker", params)
	if err != nil {
		entry.ResultError = "Host result validation unavailable"
		log.Printf("brain: desktop result capture failed: %v", err)
		return true
	}
	if r.store != nil {
		after, ok := r.store.get(entry.SessionID)
		if !ok || !bytes.Equal(before, after) {
			return false
		}
	}
	var prepared struct {
		Token string `json:"token"`
	}
	if json.Unmarshal(raw, &prepared) != nil {
		return false
	}
	if prepared.Token == "" {
		return true
	}
	commit, _ := json.Marshal(map[string]string{"token": prepared.Token})
	raw, err = r.desktopInternalCall(ctx, "internal.commitWorkerResult", commit)
	if err != nil {
		entry.ResultError = "Worker changed or result validation could not be committed"
		return false
	}
	var result struct {
		Result           string `json:"result"`
		ResultError      string `json:"resultError"`
		ReviewEvidenceID string `json:"reviewEvidenceId"`
		Instructions     string `json:"instructions"`
	}
	if json.Unmarshal(raw, &result) != nil {
		entry.ResultError = "Host result validation response was invalid"
		return true
	}
	entry.Result = result.Result
	entry.ResultError = result.ResultError
	entry.ReviewEvidenceID = result.ReviewEvidenceID
	entry.WorkflowInstructions = result.Instructions
	return true
}

func (r *registry) desktopInternalCall(ctx context.Context, method string, params json.RawMessage) (json.RawMessage, error) {
	snapshots := []json.RawMessage{}
	if r.store != nil {
		snapshots = r.store.all()
	}
	roots := r.workspaceRoots(ctx)
	if method == "desktop.readFileBytes" {
		roots = append(roots, uploads.Directory())
	}
	contextData := map[string]any{"workspaceRoots": roots, "setupRoots": r.spawnSetupRoots(ctx), "snapshots": snapshots, "daemonURL": r.cm.base}
	if method == "internal.prepareIntegration" {
		contextData["spawnCallId"], _ = ctx.Value(inboundCallIDKey{}).(string)
	}
	if method == "desktop.fleetWorkflowRequest" || method == "internal.workflowRequest" || method == "internal.prepareSpawn" || method == "desktop.managerRequestPrepare" {
		templates, err := r.handle(ctx, "library.list", json.RawMessage(`{"kind":"dispatch"}`))
		if err != nil {
			return nil, err
		}
		if method == "internal.prepareSpawn" {
			cwd, _ := ctx.Value(spawnTemplateCwdKey{}).(string)
			templates, err = json.Marshal(listLibrary(cwd, libraryFileGuardFor("library.list", cwd), libraryFilter{Kind: "dispatch"}))
			if err != nil {
				return nil, err
			}
		}

		contextData["templates"] = templates
	}
	if method == "internal.analyticsSummary" || method == "internal.analyticsRecent" {
		body, err := r.cm.listAllSessions(ctx)
		if err != nil {
			return nil, err
		}
		var rows []json.RawMessage
		if err := json.Unmarshal(body, &rows); err != nil {
			return nil, err
		}
		if rows == nil {
			rows = []json.RawMessage{}
		}
		for i, row := range rows {
			rows[i] = enrichAndCompat(row, r.meta)
		}
		contextData["analyticsSnapshots"] = rows
	}
	if method == "desktop.loadBriefBoard" || method == "desktop.moveBriefCard" {
		recent, err := r.handle(ctx, "sessions.recent", nil)
		if err != nil {
			return nil, err
		}
		contextData["recent"] = recent
	}
	return r.desktopServices.call(ctx, method, params, contextData)
}

func (r *registry) runDesktopObservations(ctx context.Context) {
	tick := time.NewTicker(2 * time.Second)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			if !r.desktopServices.available() {
				continue
			}
			if _, err := r.reconcileManagerGrants(""); err != nil {
				log.Printf("brain: manager grant sync failed: %v", err)
			}
			callCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
			_, err := r.desktopInternalCall(callCtx, "internal.observe", nil)
			cancel()
			if err != nil && ctx.Err() == nil {
				log.Printf("brain: desktop history observation failed: %v", err)
			}
		}
	}
}

// Admission precedes side effects; the accepted daemon id is recorded only
// after an acknowledged launch. Peer dispatch keeps its existing lease owner.
func (r *registry) spawn(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	if !r.desktopServices.available() {
		return r.spawnCore(ctx, raw)
	}
	var fields map[string]json.RawMessage
	if err := unmarshal(raw, &fields); err != nil {
		return nil, err
	}
	if fields == nil {
		fields = map[string]json.RawMessage{}
	}
	if remote, ok := fields["remoteOrigin"]; ok && string(remote) != "null" {
		return r.spawnCore(ctx, raw)
	}
	var p spawnParams
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	ctx = context.WithValue(ctx, spawnTemplateCwdKey{}, normalizeCwd(p.Cwd))
	fields["cwd"], _ = json.Marshal(normalizeCwd(p.Cwd))
	fields["provider"], _ = json.Marshal(r.roleProviderDefault(p))
	requested, _ := json.Marshal(map[string]any{"spawn": fields})
	prepared, err := r.desktopInternalCall(ctx, "internal.prepareSpawn", requested)
	if err != nil {
		return nil, err
	}
	var admission struct {
		Token    string                     `json:"token"`
		Cwd      string                     `json:"cwd"`
		Worktree json.RawMessage            `json:"worktree"`
		Patch    map[string]json.RawMessage `json:"patch"`
		Contract string                     `json:"contract"`
		Schema   json.RawMessage            `json:"schema"`
	}
	if err = json.Unmarshal(prepared, &admission); err != nil || admission.Token == "" {
		return nil, fmt.Errorf("invalid desktop dispatch admission")
	}
	fields["cwd"], _ = json.Marshal(admission.Cwd)
	for _, key := range []string{"cwd", "message", "resultSchema", "toolScope"} {
		if value, ok := admission.Patch[key]; ok {
			fields[key] = value
		}
	}
	amended, _ := json.Marshal(fields)
	result, err := r.spawnCore(ctx, amended, desktopSpawnMetadata{Contract: admission.Contract, Schema: admission.Schema})
	if err != nil {
		cancelled, _ := json.Marshal(map[string]string{"token": admission.Token})
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = r.desktopInternalCall(cleanupCtx, "internal.cancelSpawn", cancelled)
		return nil, err
	}
	var output map[string]json.RawMessage
	if err = json.Unmarshal(result, &output); err != nil {
		return nil, err
	}
	var sessionID string
	_ = json.Unmarshal(output["sessionId"], &sessionID)
	accepted, _ := json.Marshal(map[string]string{"token": admission.Token, "sessionId": sessionID})
	receipt, err := r.desktopInternalCall(ctx, "internal.acceptSpawn", accepted)
	if err != nil {
		// The daemon already acknowledged this launch. Returning a spawn error
		// would invite a duplicate agent; carry the failed bookkeeping explicitly.
		output["historyError"], _ = json.Marshal(err.Error())
		output["dispatchHistoryUnavailable"] = json.RawMessage("true")
	}
	var links map[string]json.RawMessage
	_ = json.Unmarshal(receipt, &links)
	for key, value := range links {
		output[key] = value
	}
	if len(admission.Worktree) > 0 && string(admission.Worktree) != "null" {
		output["worktree"] = admission.Worktree
	}
	if len(output["historyError"]) > 0 {
		output["dispatchHistoryUnavailable"] = json.RawMessage("true")
	}
	var message string
	_ = json.Unmarshal(fields["message"], &message)
	if _, rendered := admission.Patch["message"]; rendered && message != "" {
		runes := []rune(message)
		if len(runes) > 16000 {
			runes = runes[:16000]
			output["renderedMessageTruncated"] = json.RawMessage("true")
		}
		output["renderedMessage"], _ = json.Marshal(string(runes))
	}
	if message != "" && string(output["messageQueued"]) != "true" {
		ok, sendErr := r.submitMessage(ctx, sessionID, message)
		output["messageQueued"], _ = json.Marshal(ok && sendErr == nil)
		if sendErr != nil {
			output["messageError"], _ = json.Marshal(sendErr.Error())
		}
	}
	return json.Marshal(output)
}
