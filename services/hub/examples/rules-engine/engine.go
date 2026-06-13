package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"reflect"
	"regexp"
	"strings"
	"sync"
	"time"
)

// Rule is one event→action rule. See hub/docs/rules-engine-plugin.md §5.
type Rule struct {
	ID      string   `json:"id"`
	Name    string   `json:"name"`
	Enabled bool     `json:"enabled"`
	When    When     `json:"when"`
	Do      []Action `json:"do"`

	CooldownMs int64 `json:"cooldownMs,omitempty"`
	Once       bool  `json:"once,omitempty"`
}

// When is a rule trigger: an event pattern plus optional match/where conditions.
type When struct {
	Event string         `json:"event"`           // event type or wildcard, e.g. "agent.*"
	Match map[string]any `json:"match,omitempty"` // shallow equality on event.data (AND)
	Where []Cond         `json:"where,omitempty"` // richer conditions (AND)
}

// Cond is one richer condition: a dotted path into the event compared with op.
type Cond struct {
	Path  string `json:"path"`
	Op    string `json:"op"` // eq ne gt lt gte lte contains regex
	Value any    `json:"value"`
}

// Action is one thing to do when a rule fires. Fields are a union keyed by Type.
type Action struct {
	Type string `json:"type"` // notify|sendMessage|command|emit|webhook

	// notify / webhook
	Title string `json:"title,omitempty"`
	Body  string `json:"body,omitempty"`

	// sendMessage
	SessionID string `json:"sessionId,omitempty"`
	Text      string `json:"text,omitempty"`

	// command / emit
	Command string         `json:"command,omitempty"` // command: bare name, becomes command.<name>
	Event   string         `json:"event,omitempty"`   // emit: event type
	Params  map[string]any `json:"params,omitempty"`
	Data    map[string]any `json:"data,omitempty"`

	// webhook
	URL     string            `json:"url,omitempty"`
	Method  string            `json:"method,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
}

// firingsCap bounds the in-memory audit ring buffer.
const firingsCap = 100

// Firing is one audit-trail entry: a rule that fired and the actions it ran.
// Also the data payload of the rules.fired bus event.
type Firing struct {
	Time      time.Time `json:"time"`
	RuleID    string    `json:"ruleId"`
	RuleName  string    `json:"ruleName"`
	Event     string    `json:"event"`
	SessionID string    `json:"sessionId,omitempty"`
	Actions   []string  `json:"actions"`
}

// engine holds the rule set + firing state and evaluates events against it.
type engine struct {
	path      string // rules.json
	statePath string // state.json (paused flag)
	bus       *busClient
	http      *http.Client

	// jobs serializes rule evaluation onto a single worker goroutine. Crucially,
	// this keeps evaluation OFF the bus read-loop: actions make synchronous
	// capability calls (notify/sendMessage) whose result frames are read by that
	// loop, so evaluating inline would deadlock the call until it timed out.
	jobs chan Envelope

	mu        sync.RWMutex
	rules     []Rule
	paused    bool                 // global kill-switch: when true, nothing fires
	firings   []Firing             // audit ring buffer (newest appended, capped)
	lastFired map[string]time.Time // rule.id -> last fire time (cooldown)
	fired     map[string]bool      // rule.id+sessionId -> fired (once)
}

func newEngine(path, statePath string) *engine {
	return &engine{
		path:      path,
		statePath: statePath,
		http:      &http.Client{Timeout: 10 * time.Second},
		jobs:      make(chan Envelope, 256),
		lastFired: map[string]time.Time{},
		fired:     map[string]bool{},
	}
}

// run is the evaluation worker: it drains the jobs queue, evaluating one event
// at a time. Start it once (go eng.run(ctx)).
func (e *engine) run(ctx context.Context) {
	for {
		select {
		case <-ctx.Done():
			return
		case ev := <-e.jobs:
			e.evaluate(ev.Type, envelopeToMap(ev))
		}
	}
}

// enqueue hands an event to the worker. Non-blocking: if the queue is full
// (a slow capability is back-pressuring), drop and log rather than stall the
// bus read-loop or poll loop.
func (e *engine) enqueue(ev Envelope) {
	select {
	case e.jobs <- ev:
	default:
		log.Printf("rules-engine: event queue full, dropped %s", ev.Type)
	}
}

func (e *engine) load() error {
	data, err := os.ReadFile(e.path)
	if err != nil {
		return err
	}
	var rules []Rule
	if err := json.Unmarshal(data, &rules); err != nil {
		return err
	}
	e.mu.Lock()
	e.rules = rules
	e.mu.Unlock()
	return nil
}

func (e *engine) save() error {
	e.mu.RLock()
	data, err := json.MarshalIndent(e.rules, "", "  ")
	e.mu.RUnlock()
	if err != nil {
		return err
	}
	return os.WriteFile(e.path, append(data, '\n'), 0o644)
}

func (e *engine) count() int {
	e.mu.RLock()
	defer e.mu.RUnlock()
	return len(e.rules)
}

// engineState is the small persisted control state (the kill-switch).
type engineState struct {
	Paused bool `json:"paused"`
}

func (e *engine) loadState() {
	data, err := os.ReadFile(e.statePath)
	if err != nil {
		return // no state file yet — default unpaused
	}
	var s engineState
	if json.Unmarshal(data, &s) == nil {
		e.mu.Lock()
		e.paused = s.Paused
		e.mu.Unlock()
	}
}

func (e *engine) saveState() {
	e.mu.RLock()
	data, _ := json.Marshal(engineState{Paused: e.paused})
	e.mu.RUnlock()
	if err := os.WriteFile(e.statePath, append(data, '\n'), 0o644); err != nil {
		log.Printf("rules-engine: could not persist state: %v", err)
	}
}

func (e *engine) isPaused() bool {
	e.mu.RLock()
	defer e.mu.RUnlock()
	return e.paused
}

func (e *engine) setPaused(p bool) {
	e.mu.Lock()
	e.paused = p
	e.mu.Unlock()
	e.saveState()
	log.Printf("rules-engine: paused=%v", p)
}

// recentFirings returns the audit buffer newest-first.
func (e *engine) recentFirings() []Firing {
	e.mu.RLock()
	defer e.mu.RUnlock()
	out := make([]Firing, len(e.firings))
	for i, f := range e.firings {
		out[len(e.firings)-1-i] = f
	}
	return out
}

// onEvent feeds a real bus event through the rule loop (called by the bus
// read-loop; enqueues so evaluation happens off that goroutine).
func (e *engine) onEvent(ev Envelope) { e.enqueue(ev) }

// pollLoop drives cost/usage rules: every interval it calls agents.list and
// feeds one synthetic "agents.poll" event per agent through the worker. The
// agents.list call runs on this goroutine (not the read-loop), so its result
// can be read while we wait.
func (e *engine) pollLoop(ctx context.Context, interval time.Duration) {
	t := time.NewTicker(interval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			if e.bus == nil {
				continue
			}
			res, err := e.bus.call("agents.list", json.RawMessage(`{}`))
			if err != nil {
				continue // app side may be down; quietly retry next tick
			}
			var agents []json.RawMessage
			if err := json.Unmarshal(res, &agents); err != nil {
				continue
			}
			for _, a := range agents {
				e.enqueue(Envelope{Type: "agents.poll", Source: source, Data: a})
			}
		}
	}
}

// evaluate runs every enabled rule against one event (real or synthetic).
func (e *engine) evaluate(eventType string, root map[string]any) {
	data, _ := root["data"].(map[string]any)

	e.mu.RLock()
	if e.paused { // global kill-switch: consume the event but fire nothing
		e.mu.RUnlock()
		return
	}
	rules := make([]Rule, len(e.rules))
	copy(rules, e.rules)
	e.mu.RUnlock()

	now := time.Now()
	for _, r := range rules {
		if !r.Enabled {
			continue
		}
		if !topicMatch(r.When.Event, eventType) {
			continue
		}
		if !matchEquals(r.When.Match, data) {
			continue
		}
		if !whereAll(r.When.Where, root) {
			continue
		}

		sessionID, _ := data["sessionId"].(string)
		key := r.ID + "|" + sessionID

		e.mu.Lock()
		if r.Once && e.fired[key] {
			e.mu.Unlock()
			continue
		}
		if r.CooldownMs > 0 {
			if last, ok := e.lastFired[r.ID]; ok && now.Sub(last) < time.Duration(r.CooldownMs)*time.Millisecond {
				e.mu.Unlock()
				continue
			}
		}
		e.lastFired[r.ID] = now
		if r.Once {
			e.fired[key] = true
		}
		e.mu.Unlock()

		log.Printf("rules-engine: rule %q fired on %s", r.ID, eventType)
		e.recordFiring(r, eventType, sessionID)
		for _, a := range r.Do {
			e.runAction(a, root)
		}
	}
}

// recordFiring appends to the audit ring buffer and emits a rules.fired bus
// event so dashboards (and the editor's "recent firings" panel) can observe it.
func (e *engine) recordFiring(r Rule, eventType, sessionID string) {
	types := make([]string, len(r.Do))
	for i, a := range r.Do {
		types[i] = a.Type
	}
	f := Firing{Time: time.Now(), RuleID: r.ID, RuleName: r.Name, Event: eventType, SessionID: sessionID, Actions: types}

	e.mu.Lock()
	e.firings = append(e.firings, f)
	if len(e.firings) > firingsCap {
		e.firings = e.firings[len(e.firings)-firingsCap:]
	}
	e.mu.Unlock()

	if e.bus != nil {
		e.bus.publish("rules.fired", mustJSON(f))
	}
}

// runAction executes one action, interpolating {{path}} templates against root.
func (e *engine) runAction(a Action, root map[string]any) {
	switch a.Type {
	case "notify":
		params := mustJSON(map[string]any{
			"title": tmpl(a.Title, root),
			"body":  tmpl(a.Body, root),
		})
		if _, err := e.bus.call("notifications.post", params); err != nil {
			log.Printf("rules-engine: notify failed: %v", err)
		}
	case "sendMessage":
		params := mustJSON(map[string]any{
			"sessionId": tmpl(a.SessionID, root),
			"text":      tmpl(a.Text, root),
		})
		if _, err := e.bus.call("agents.sendMessage", params); err != nil {
			log.Printf("rules-engine: sendMessage failed: %v", err)
		}
	case "command":
		e.bus.publish("command."+a.Command, mustJSON(tmplMap(a.Params, root)))
	case "emit":
		e.bus.publish(tmpl(a.Event, root), mustJSON(tmplMap(a.Data, root)))
	case "webhook":
		e.runWebhook(a, root)
	default:
		log.Printf("rules-engine: unknown action type %q", a.Type)
	}
}

func (e *engine) runWebhook(a Action, root map[string]any) {
	method := a.Method
	if method == "" {
		method = http.MethodPost
	}
	url := tmpl(a.URL, root)
	var body io.Reader
	if a.Body != "" {
		body = strings.NewReader(tmpl(a.Body, root))
	}
	req, err := http.NewRequest(method, url, body)
	if err != nil {
		log.Printf("rules-engine: webhook build failed: %v", err)
		return
	}
	for k, v := range a.Headers {
		req.Header.Set(k, tmpl(v, root))
	}
	resp, err := e.http.Do(req)
	if err != nil {
		log.Printf("rules-engine: webhook %s failed: %v", url, err)
		return
	}
	_, _ = io.Copy(io.Discard, resp.Body)
	resp.Body.Close()
}

// handleRules serves GET (current list) and PUT (replace + persist) of /rules.
func (e *engine) handleRules(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		e.mu.RLock()
		rules := e.rules
		e.mu.RUnlock()
		writeJSON(w, http.StatusOK, rules)
	case http.MethodPut:
		var rules []Rule
		if err := json.NewDecoder(r.Body).Decode(&rules); err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]any{"error": err.Error()})
			return
		}
		e.mu.Lock()
		e.rules = rules
		// Reset firing state so edited rules behave predictably.
		e.lastFired = map[string]time.Time{}
		e.fired = map[string]bool{}
		e.mu.Unlock()
		if err := e.save(); err != nil {
			writeJSON(w, http.StatusInternalServerError, map[string]any{"error": err.Error()})
			return
		}
		log.Printf("rules-engine: rules replaced (%d) and persisted", len(rules))
		writeJSON(w, http.StatusOK, map[string]any{"ok": true, "count": len(rules)})
	default:
		w.Header().Set("Allow", "GET, PUT")
		w.WriteHeader(http.StatusMethodNotAllowed)
	}
}

// handleState serves GET (current paused flag) and PUT (set it) for the global
// kill-switch.
func (e *engine) handleState(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		writeJSON(w, http.StatusOK, engineState{Paused: e.isPaused()})
	case http.MethodPut:
		var s engineState
		if err := json.NewDecoder(r.Body).Decode(&s); err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]any{"error": err.Error()})
			return
		}
		e.setPaused(s.Paused)
		writeJSON(w, http.StatusOK, map[string]any{"ok": true, "paused": s.Paused})
	default:
		w.Header().Set("Allow", "GET, PUT")
		w.WriteHeader(http.StatusMethodNotAllowed)
	}
}

// handleLog serves the audit ring buffer (recent rule firings, newest-first).
func (e *engine) handleLog(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, e.recentFirings())
}

// ── matching + templating helpers ──────────────────────────────────────────

// topicMatch mirrors the hub's topic patterns: exact, "ns.*" wildcard, or "*".
func topicMatch(pattern, topic string) bool {
	if pattern == "" || pattern == "*" {
		return true
	}
	if strings.HasSuffix(pattern, ".*") {
		return strings.HasPrefix(topic, pattern[:len(pattern)-1]) // keep the dot
	}
	return pattern == topic
}

// matchEquals checks shallow key/value equality of match against data (AND).
func matchEquals(match map[string]any, data map[string]any) bool {
	for k, want := range match {
		got, ok := data[k]
		if !ok || !valueEquals(want, got) {
			return false
		}
	}
	return true
}

func whereAll(conds []Cond, root map[string]any) bool {
	for _, c := range conds {
		if !evalCond(c, root) {
			return false
		}
	}
	return true
}

func evalCond(c Cond, root map[string]any) bool {
	got, ok := resolvePath(root, c.Path)
	switch c.Op {
	case "eq":
		return ok && valueEquals(c.Value, got)
	case "ne":
		return !ok || !valueEquals(c.Value, got)
	case "gt", "lt", "gte", "lte":
		gf, ok1 := toFloat(got)
		wf, ok2 := toFloat(c.Value)
		if !ok || !ok1 || !ok2 {
			return false
		}
		switch c.Op {
		case "gt":
			return gf > wf
		case "lt":
			return gf < wf
		case "gte":
			return gf >= wf
		case "lte":
			return gf <= wf
		}
	case "contains":
		return ok && strings.Contains(toString(got), toString(c.Value))
	case "regex":
		if !ok {
			return false
		}
		re, err := regexp.Compile(toString(c.Value))
		return err == nil && re.MatchString(toString(got))
	}
	return false
}

// valueEquals compares two JSON-decoded values. Numbers are normalized so
// 5 (int from a literal) and 5.0 (float64 from JSON) compare equal.
func valueEquals(a, b any) bool {
	if af, ok := toFloat(a); ok {
		if bf, ok := toFloat(b); ok {
			return af == bf
		}
	}
	return reflect.DeepEqual(a, b)
}

func toFloat(v any) (float64, bool) {
	switch n := v.(type) {
	case float64:
		return n, true
	case float32:
		return float64(n), true
	case int:
		return float64(n), true
	case int64:
		return float64(n), true
	case json.Number:
		f, err := n.Float64()
		return f, err == nil
	}
	return 0, false
}

func toString(v any) string {
	if s, ok := v.(string); ok {
		return s
	}
	return fmt.Sprintf("%v", v)
}

// resolvePath dots into a nested map, e.g. "data.costUSD" or "type".
func resolvePath(root map[string]any, path string) (any, bool) {
	cur := any(root)
	for _, part := range strings.Split(path, ".") {
		m, ok := cur.(map[string]any)
		if !ok {
			return nil, false
		}
		cur, ok = m[part]
		if !ok {
			return nil, false
		}
	}
	return cur, true
}

var tmplRe = regexp.MustCompile(`{{\s*([^}]+?)\s*}}`)

// tmpl interpolates {{path}} references in s against root.
func tmpl(s string, root map[string]any) string {
	if !strings.Contains(s, "{{") {
		return s
	}
	return tmplRe.ReplaceAllStringFunc(s, func(m string) string {
		path := strings.TrimSpace(tmplRe.FindStringSubmatch(m)[1])
		if v, ok := resolvePath(root, path); ok {
			return toString(v)
		}
		return ""
	})
}

// tmplMap interpolates string values of a params/data map (one level deep,
// recursing into nested maps). Non-string scalars pass through unchanged.
func tmplMap(m map[string]any, root map[string]any) map[string]any {
	if m == nil {
		return map[string]any{}
	}
	out := make(map[string]any, len(m))
	for k, v := range m {
		switch vv := v.(type) {
		case string:
			out[k] = tmpl(vv, root)
		case map[string]any:
			out[k] = tmplMap(vv, root)
		default:
			out[k] = v
		}
	}
	return out
}

// envelopeToMap turns an event envelope into the {type,source,data,…} map that
// templates and where-conditions resolve against.
func envelopeToMap(ev Envelope) map[string]any {
	root := map[string]any{
		"id":     ev.ID,
		"type":   ev.Type,
		"source": ev.Source,
	}
	if len(ev.Data) > 0 {
		var d any
		if err := json.Unmarshal(ev.Data, &d); err == nil {
			root["data"] = d
		}
	}
	if _, ok := root["data"]; !ok {
		root["data"] = map[string]any{}
	}
	return root
}

func mustJSON(v any) json.RawMessage {
	data, err := json.Marshal(v)
	if err != nil {
		return json.RawMessage(`{}`)
	}
	return data
}
