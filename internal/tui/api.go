package tui

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/djtouchette/workspacer/internal/queue"
	"github.com/djtouchette/workspacer/internal/session"
	"github.com/djtouchette/workspacer/internal/workflow"
)

// APIClient talks to the web API so the TUI doesn't need direct callbacks.
type APIClient struct {
	BaseURL string
	client  *http.Client
}

// NewAPIClient creates an HTTP client pointed at the given base URL.
func NewAPIClient(baseURL string) *APIClient {
	return &APIClient{
		BaseURL: baseURL,
		client:  &http.Client{Timeout: 10 * time.Second},
	}
}

// ── Public methods ──

// GetSession fetches a single session by ID.
func (c *APIClient) GetSession(id string) (*session.Session, error) {
	var s session.Session
	if err := c.get(fmt.Sprintf("/api/sessions/%s", id), &s); err != nil {
		return nil, err
	}
	return &s, nil
}

// GetAllSessions fetches every session from the web API.
func (c *APIClient) GetAllSessions() ([]*session.Session, error) {
	var sessions []*session.Session
	if err := c.get("/api/sessions", &sessions); err != nil {
		return nil, err
	}
	return sessions, nil
}

// GetTasks fetches the task queue.
func (c *APIClient) GetTasks() ([]*queue.Task, error) {
	var tasks []*queue.Task
	if err := c.get("/api/queue", &tasks); err != nil {
		return nil, err
	}
	return tasks, nil
}

// GetWorkflowRuns fetches all workflow runs.
func (c *APIClient) GetWorkflowRuns() ([]*workflow.WorkflowRun, error) {
	var runs []*workflow.WorkflowRun
	if err := c.get("/api/workflow-runs", &runs); err != nil {
		return nil, err
	}
	return runs, nil
}

// DeleteSession removes a session.
func (c *APIClient) DeleteSession(sessionID string) error {
	req, err := http.NewRequest("DELETE", fmt.Sprintf("%s/api/sessions/%s", c.BaseURL, sessionID), nil)
	if err != nil {
		return err
	}
	resp, err := c.client.Do(req)
	if err != nil {
		return err
	}
	resp.Body.Close()
	return nil
}

// Approve approves a pending tool call for a session.
func (c *APIClient) Approve(sessionID string) error {
	return c.post(fmt.Sprintf("/api/sessions/%s/approve", sessionID), nil)
}

// Deny denies a pending tool call for a session.
func (c *APIClient) Deny(sessionID string) error {
	return c.post(fmt.Sprintf("/api/sessions/%s/deny", sessionID), nil)
}

// NewChat starts a new interactive chat session.
func (c *APIClient) NewChat(cwd, prompt string) error {
	return c.post("/api/sessions/new", map[string]string{
		"cwd":    cwd,
		"prompt": prompt,
	})
}

// AddTask queues a new task.
func (c *APIClient) AddTask(title, prompt, cwd, autonomy string, budgetDollars float64) error {
	return c.post("/api/queue", map[string]any{
		"title":         title,
		"prompt":        prompt,
		"cwd":           cwd,
		"autonomy":      autonomy,
		"budgetDollars": budgetDollars,
	})
}

// SendMessage sends a follow-up message to an active session.
func (c *APIClient) SendMessage(sessionID, message string) error {
	return c.post(fmt.Sprintf("/api/sessions/%s/message", sessionID), map[string]string{
		"message": message,
	})
}

// ResumeSession resumes an ended session with a new prompt.
func (c *APIClient) ResumeSession(sessionID, prompt string) error {
	return c.post(fmt.Sprintf("/api/sessions/%s/resume", sessionID), map[string]string{
		"prompt": prompt,
	})
}

// ── SSE streaming ──

// ConnectSSE connects to the /events SSE endpoint and sends bubbletea messages
// for each update. Call this in a goroutine. It reconnects on failure.
func (c *APIClient) ConnectSSE(send func(msg any)) {
	for {
		c.streamSSE(send)
		// Reconnect after 1 second
		time.Sleep(time.Second)
	}
}

func (c *APIClient) streamSSE(send func(msg any)) {
	resp, err := c.client.Get(c.BaseURL + "/events")
	if err != nil {
		return
	}
	defer resp.Body.Close()

	scanner := bufio.NewScanner(resp.Body)
	scanner.Buffer(make([]byte, 1024*1024), 1024*1024)

	for scanner.Scan() {
		line := scanner.Text()
		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		data := []byte(strings.TrimPrefix(line, "data: "))

		var envelope struct {
			Type     string              `json:"type"`
			Data     json.RawMessage     `json:"data"`
			Sessions json.RawMessage     `json:"sessions"`
			Tasks    json.RawMessage     `json:"tasks"`
		}
		if err := json.Unmarshal(data, &envelope); err != nil {
			continue
		}

		switch envelope.Type {
		case "init":
			var sessions []*session.Session
			var tasks []*queue.Task
			json.Unmarshal(envelope.Sessions, &sessions)
			json.Unmarshal(envelope.Tasks, &tasks)
			send(SessionsUpdatedMsg{Sessions: sessions})
			send(TasksUpdatedMsg{Tasks: tasks})

		case "session_update":
			var sess session.Session
			if json.Unmarshal(envelope.Data, &sess) == nil {
				// Send full session list refresh for simplicity
				// (single session update requires merge logic, poll is easier)
				go func() {
					if all, err := c.GetAllSessions(); err == nil {
						send(SessionsUpdatedMsg{Sessions: all})
					}
				}()
			}

		case "queue_update":
			var tasks []*queue.Task
			if json.Unmarshal(envelope.Data, &tasks) == nil {
				send(TasksUpdatedMsg{Tasks: tasks})
			}
		}
	}
}

// ── Private helpers ──

func (c *APIClient) get(path string, result any) error {
	resp, err := c.client.Get(c.BaseURL + path)
	if err != nil {
		log.Printf("[tui-api] GET %s error: %v", path, err)
		return err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		log.Printf("[tui-api] GET %s read error: %v", path, err)
		return err
	}

	if resp.StatusCode >= 400 {
		log.Printf("[tui-api] GET %s status %d: %s", path, resp.StatusCode, string(body))
		return fmt.Errorf("API error %d: %s", resp.StatusCode, string(body))
	}

	if result != nil {
		if err := json.Unmarshal(body, result); err != nil {
			log.Printf("[tui-api] GET %s decode error: %v", path, err)
			return err
		}
	}
	return nil
}

func (c *APIClient) post(path string, payload any) error {
	var bodyReader io.Reader
	if payload != nil {
		data, err := json.Marshal(payload)
		if err != nil {
			log.Printf("[tui-api] POST %s marshal error: %v", path, err)
			return err
		}
		bodyReader = bytes.NewReader(data)
	}

	resp, err := c.client.Post(c.BaseURL+path, "application/json", bodyReader)
	if err != nil {
		log.Printf("[tui-api] POST %s error: %v", path, err)
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode >= 400 {
		body, _ := io.ReadAll(resp.Body)
		log.Printf("[tui-api] POST %s status %d: %s", path, resp.StatusCode, string(body))
		return fmt.Errorf("API error %d: %s", resp.StatusCode, string(body))
	}
	return nil
}
