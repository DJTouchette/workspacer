// Package workflow manages multi-step workflow definitions and running instances.
package workflow

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// Variable defines an input parameter for a workflow step.
type Variable struct {
	Name    string   `json:"name"`
	Label   string   `json:"label"`
	Type    string   `json:"type"`              // "text", "textarea", "select"
	Source  string   `json:"source"`            // "user" (manual input), "auto" (injected)
	Default string   `json:"default,omitempty"`
	Options []string `json:"options,omitempty"` // for select type
}

// Step defines a single step within a workflow.
type Step struct {
	Name      string     `json:"name"`
	Prompt    string     `json:"prompt"`
	Autonomy  string     `json:"autonomy"` // manual, semi, full
	Variables []Variable `json:"variables,omitempty"`
}

// Workflow defines a reusable multi-step task pipeline.
type Workflow struct {
	ID          string    `json:"id"`
	Name        string    `json:"name"`
	Description string    `json:"description"`
	Steps       []Step    `json:"steps"`
	IsBuiltIn   bool      `json:"isBuiltIn"`
	CreatedAt   time.Time `json:"createdAt"`
}

// WorkflowRun tracks a running workflow instance.
type WorkflowRun struct {
	ID           string            `json:"id"`
	WorkflowID   string            `json:"workflowId"`
	WorkflowName string            `json:"workflowName"`
	ProjectPath  string            `json:"projectPath"`
	Steps        []StepRun         `json:"steps"`
	Variables    map[string]string `json:"variables"`
	Status       string            `json:"status"` // running, completed, failed, cancelled
	CurrentStep  int               `json:"currentStep"`
	CreatedAt    time.Time         `json:"createdAt"`
}

// StepRun tracks the execution state of a single workflow step.
type StepRun struct {
	Name      string `json:"name"`
	TaskID    string `json:"taskId,omitempty"`
	SessionID string `json:"sessionId,omitempty"`
	Status    string `json:"status"` // pending, running, completed, failed, skipped
	Summary   string `json:"summary,omitempty"`
}

// LaunchStepFunc creates a task via the queue and returns the task ID.
type LaunchStepFunc func(cwd, prompt, autonomy string) (taskID string, err error)

// builtInWorkflows are always available and cannot be modified or deleted.
var builtInWorkflows = []*Workflow{
	{
		ID:          "full-feature",
		Name:        "Full Feature",
		Description: "Implement a feature with tests and code review",
		IsBuiltIn:   true,
		Steps: []Step{
			{
				Name:     "Implement",
				Prompt:   "{{feature_description}}",
				Autonomy: "semi",
				Variables: []Variable{
					{Name: "feature_description", Label: "Feature Description", Type: "textarea", Source: "user"},
				},
			},
			{
				Name:     "Test",
				Prompt:   "Write comprehensive tests for the changes made in the previous step.\n\nPrevious step summary:\n{{previous_summary}}",
				Autonomy: "semi",
			},
			{
				Name:     "Review",
				Prompt:   "Review the implementation and tests from the previous steps. Check for bugs, edge cases, security issues, and code quality.\n\nPrevious steps:\n{{all_summaries}}",
				Autonomy: "manual",
			},
		},
	},
	{
		ID:          "bug-fix",
		Name:        "Bug Fix",
		Description: "Investigate, fix, and verify a bug",
		IsBuiltIn:   true,
		Steps: []Step{
			{
				Name:     "Investigate",
				Prompt:   "Investigate this bug: {{bug_description}}\n\nFind the root cause.",
				Autonomy: "semi",
				Variables: []Variable{
					{Name: "bug_description", Label: "Bug Description", Type: "textarea", Source: "user"},
				},
			},
			{
				Name:     "Fix",
				Prompt:   "Fix the bug identified in the investigation.\n\nInvestigation summary:\n{{previous_summary}}",
				Autonomy: "semi",
			},
			{
				Name:     "Verify",
				Prompt:   "Verify the fix by running tests and checking edge cases.\n\nFix summary:\n{{previous_summary}}",
				Autonomy: "semi",
			},
		},
	},
	{
		ID:          "refactor",
		Name:        "Refactor",
		Description: "Audit, refactor, and test code improvements",
		IsBuiltIn:   true,
		Steps: []Step{
			{
				Name:     "Audit",
				Prompt:   "Audit {{target}} for code quality, maintainability, and potential improvements.",
				Autonomy: "manual",
				Variables: []Variable{
					{Name: "target", Label: "Refactor Target", Type: "text", Source: "user"},
				},
			},
			{
				Name:     "Refactor",
				Prompt:   "Refactor based on the audit findings. Do not change behavior.\n\nAudit findings:\n{{previous_summary}}",
				Autonomy: "semi",
			},
			{
				Name:     "Test",
				Prompt:   "Run and update tests to verify the refactoring didn't break anything.\n\nRefactoring summary:\n{{previous_summary}}",
				Autonomy: "semi",
			},
		},
	},
}

// Store manages workflow definitions and running instances.
type Store struct {
	mu        sync.RWMutex
	workflows map[string]*Workflow
	runs      map[string]*WorkflowRun
	dataDir   string

	// LaunchStep creates a task via the queue. Set by the caller during wiring.
	LaunchStep LaunchStepFunc
}

// NewStore creates a workflow store, loading persisted data and seeding built-ins.
func NewStore(dataDir string) *Store {
	s := &Store{
		workflows: make(map[string]*Workflow),
		runs:      make(map[string]*WorkflowRun),
		dataDir:   dataDir,
	}

	// Seed built-in workflows
	for _, w := range builtInWorkflows {
		cp := *w
		s.workflows[cp.ID] = &cp
	}

	// Load persisted workflows and runs from disk
	if dataDir != "" {
		os.MkdirAll(filepath.Join(dataDir, "workflows"), 0755)
		os.MkdirAll(filepath.Join(dataDir, "workflow-runs"), 0755)
		s.loadWorkflows()
		s.loadRuns()
	}

	return s
}

// ── Workflow CRUD ──

// List returns all workflow definitions.
func (s *Store) List() []Workflow {
	s.mu.RLock()
	defer s.mu.RUnlock()

	out := make([]Workflow, 0, len(s.workflows))
	for _, w := range s.workflows {
		out = append(out, *w)
	}
	return out
}

// Get returns a workflow by ID, or nil if not found.
func (s *Store) Get(id string) *Workflow {
	s.mu.RLock()
	defer s.mu.RUnlock()

	w := s.workflows[id]
	if w == nil {
		return nil
	}
	cp := *w
	return &cp
}

// Create adds a new user workflow, generating an ID and setting CreatedAt.
func (s *Store) Create(w Workflow) Workflow {
	s.mu.Lock()
	defer s.mu.Unlock()

	w.ID = generateID("wf-")
	w.CreatedAt = time.Now()
	w.IsBuiltIn = false

	cp := w
	s.workflows[w.ID] = &cp
	s.saveWorkflow(&cp)

	return w
}

// Delete removes a user workflow. Returns an error if built-in or not found.
func (s *Store) Delete(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	existing := s.workflows[id]
	if existing == nil {
		return fmt.Errorf("workflow %q not found", id)
	}
	if existing.IsBuiltIn {
		return fmt.Errorf("cannot delete built-in workflow %q", id)
	}

	delete(s.workflows, id)

	if s.dataDir != "" {
		path := filepath.Join(s.dataDir, "workflows", id+".json")
		os.Remove(path)
	}

	return nil
}

// ── Run management ──

// StartRun creates a new workflow run and launches the first step.
func (s *Store) StartRun(workflowID, projectPath string, variables map[string]string) (*WorkflowRun, error) {
	s.mu.RLock()
	wf := s.workflows[workflowID]
	s.mu.RUnlock()

	if wf == nil {
		return nil, fmt.Errorf("workflow %q not found", workflowID)
	}
	if s.LaunchStep == nil {
		return nil, fmt.Errorf("LaunchStep callback not configured")
	}

	run := &WorkflowRun{
		ID:           generateID("wr-"),
		WorkflowID:   wf.ID,
		WorkflowName: wf.Name,
		ProjectPath:  projectPath,
		Variables:    variables,
		Status:       "running",
		CurrentStep:  0,
		CreatedAt:    time.Now(),
	}

	// Initialize step runs
	for _, step := range wf.Steps {
		run.Steps = append(run.Steps, StepRun{
			Name:   step.Name,
			Status: "pending",
		})
	}

	s.mu.Lock()
	s.runs[run.ID] = run
	s.mu.Unlock()

	// Launch the first step
	if err := s.launchCurrentStep(run, wf); err != nil {
		s.mu.Lock()
		run.Status = "failed"
		run.Steps[0].Status = "failed"
		s.mu.Unlock()
		s.saveRun(run)
		return run, fmt.Errorf("failed to launch first step: %w", err)
	}

	s.saveRun(run)
	log.Printf("[workflow] started run=%s workflow=%s project=%s", run.ID, wf.Name, projectPath)
	return run, nil
}

// AdvanceRun marks the current step as complete and starts the next one.
func (s *Store) AdvanceRun(runID string, stepSummary string) error {
	s.mu.Lock()
	run := s.runs[runID]
	if run == nil {
		s.mu.Unlock()
		return fmt.Errorf("run %q not found", runID)
	}
	if run.Status != "running" {
		s.mu.Unlock()
		return fmt.Errorf("run %q is not running (status=%s)", runID, run.Status)
	}

	// Mark current step complete
	cur := run.CurrentStep
	if cur < len(run.Steps) {
		run.Steps[cur].Status = "completed"
		run.Steps[cur].Summary = stepSummary
	}

	// Advance to next step
	run.CurrentStep++

	if run.CurrentStep >= len(run.Steps) {
		// All steps done
		run.Status = "completed"
		s.mu.Unlock()
		s.saveRun(run)
		log.Printf("[workflow] run=%s completed all %d steps", runID, len(run.Steps))
		return nil
	}
	s.mu.Unlock()

	// Look up workflow for step definitions
	wf := s.Get(run.WorkflowID)
	if wf == nil {
		s.mu.Lock()
		run.Status = "failed"
		s.mu.Unlock()
		s.saveRun(run)
		return fmt.Errorf("workflow %q not found for run %q", run.WorkflowID, runID)
	}

	// Launch the next step
	if err := s.launchCurrentStep(run, wf); err != nil {
		s.mu.Lock()
		run.Status = "failed"
		run.Steps[run.CurrentStep].Status = "failed"
		s.mu.Unlock()
		s.saveRun(run)
		return fmt.Errorf("failed to launch step %d: %w", run.CurrentStep, err)
	}

	s.saveRun(run)
	log.Printf("[workflow] run=%s advanced to step %d/%d (%s)", runID, run.CurrentStep+1, len(run.Steps), run.Steps[run.CurrentStep].Name)
	return nil
}

// GetRun returns a workflow run by ID, or nil if not found.
func (s *Store) GetRun(id string) *WorkflowRun {
	s.mu.RLock()
	defer s.mu.RUnlock()

	run := s.runs[id]
	if run == nil {
		return nil
	}
	cp := *run
	return &cp
}

// GetRuns returns all workflow runs.
func (s *Store) GetRuns() []*WorkflowRun {
	s.mu.RLock()
	defer s.mu.RUnlock()

	out := make([]*WorkflowRun, 0, len(s.runs))
	for _, run := range s.runs {
		cp := *run
		out = append(out, &cp)
	}
	return out
}

// CancelRun cancels a running workflow, skipping remaining steps.
func (s *Store) CancelRun(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	run := s.runs[id]
	if run == nil {
		return fmt.Errorf("run %q not found", id)
	}
	if run.Status != "running" {
		return fmt.Errorf("run %q is not running (status=%s)", id, run.Status)
	}

	run.Status = "cancelled"
	for i := range run.Steps {
		if run.Steps[i].Status == "pending" || run.Steps[i].Status == "running" {
			run.Steps[i].Status = "skipped"
		}
	}

	s.saveRun(run)
	log.Printf("[workflow] cancelled run=%s", id)
	return nil
}

// FindRunByTaskID finds the workflow run that contains the given task ID.
func (s *Store) FindRunByTaskID(taskID string) *WorkflowRun {
	s.mu.RLock()
	defer s.mu.RUnlock()

	for _, run := range s.runs {
		for _, step := range run.Steps {
			if step.TaskID == taskID {
				cp := *run
				return &cp
			}
		}
	}
	return nil
}

// SetStepSessionID records the session ID for a step's task.
func (s *Store) SetStepSessionID(taskID, sessionID string) {
	s.mu.Lock()
	defer s.mu.Unlock()

	for _, run := range s.runs {
		for i := range run.Steps {
			if run.Steps[i].TaskID == taskID {
				run.Steps[i].SessionID = sessionID
				s.saveRun(run)
				return
			}
		}
	}
}

// ── Internal helpers ──

// launchCurrentStep interpolates the prompt and launches the step via the queue.
// Caller must NOT hold s.mu when calling this method.
func (s *Store) launchCurrentStep(run *WorkflowRun, wf *Workflow) error {
	stepIdx := run.CurrentStep
	if stepIdx >= len(wf.Steps) {
		return fmt.Errorf("step index %d out of range", stepIdx)
	}

	stepDef := wf.Steps[stepIdx]

	// Collect previous summaries
	var previousSummaries []string
	for i := 0; i < stepIdx; i++ {
		if run.Steps[i].Summary != "" {
			previousSummaries = append(previousSummaries, fmt.Sprintf("Step %d (%s): %s", i+1, run.Steps[i].Name, run.Steps[i].Summary))
		}
	}

	// Build variables map with auto-injected values
	vars := make(map[string]string)
	for k, v := range run.Variables {
		vars[k] = v
	}
	vars["project_name"] = filepath.Base(run.ProjectPath)

	prompt := interpolatePrompt(stepDef.Prompt, vars, previousSummaries)

	taskID, err := s.LaunchStep(run.ProjectPath, prompt, stepDef.Autonomy)
	if err != nil {
		return err
	}

	s.mu.Lock()
	run.Steps[stepIdx].TaskID = taskID
	run.Steps[stepIdx].Status = "running"
	s.mu.Unlock()

	return nil
}

// interpolatePrompt replaces {{...}} patterns in a prompt template.
func interpolatePrompt(prompt string, variables map[string]string, previousSummaries []string) string {
	result := prompt

	// Replace {{previous_summary}} with the last summary
	if len(previousSummaries) > 0 {
		result = strings.ReplaceAll(result, "{{previous_summary}}", previousSummaries[len(previousSummaries)-1])
	} else {
		result = strings.ReplaceAll(result, "{{previous_summary}}", "(no previous summary available)")
	}

	// Replace {{all_summaries}} with all summaries combined
	if len(previousSummaries) > 0 {
		result = strings.ReplaceAll(result, "{{all_summaries}}", strings.Join(previousSummaries, "\n\n"))
	} else {
		result = strings.ReplaceAll(result, "{{all_summaries}}", "(no previous summaries available)")
	}

	// Replace all user-provided variables
	for name, value := range variables {
		result = strings.ReplaceAll(result, "{{"+name+"}}", value)
	}

	return result
}

// ── Persistence ──

func (s *Store) saveWorkflow(w *Workflow) {
	if s.dataDir == "" || w.IsBuiltIn {
		return
	}
	path := filepath.Join(s.dataDir, "workflows", w.ID+".json")
	data, err := json.Marshal(w)
	if err != nil {
		log.Printf("[workflow] save error id=%s: %v", w.ID, err)
		return
	}
	if err := os.WriteFile(path, data, 0644); err != nil {
		log.Printf("[workflow] write error id=%s: %v", w.ID, err)
	}
}

func (s *Store) saveRun(run *WorkflowRun) {
	if s.dataDir == "" {
		return
	}
	path := filepath.Join(s.dataDir, "workflow-runs", run.ID+".json")
	data, err := json.Marshal(run)
	if err != nil {
		log.Printf("[workflow] save run error id=%s: %v", run.ID, err)
		return
	}
	if err := os.WriteFile(path, data, 0644); err != nil {
		log.Printf("[workflow] write run error id=%s: %v", run.ID, err)
	}
}

func (s *Store) loadWorkflows() {
	dir := filepath.Join(s.dataDir, "workflows")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	loaded := 0
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, e.Name()))
		if err != nil {
			continue
		}
		var w Workflow
		if err := json.Unmarshal(data, &w); err != nil {
			continue
		}
		// Don't overwrite built-in workflows from disk
		if _, exists := s.workflows[w.ID]; exists && s.workflows[w.ID].IsBuiltIn {
			continue
		}
		s.workflows[w.ID] = &w
		loaded++
	}
	if loaded > 0 {
		log.Printf("[workflow] loaded %d persisted workflows", loaded)
	}
}

func (s *Store) loadRuns() {
	dir := filepath.Join(s.dataDir, "workflow-runs")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	loaded := 0
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, e.Name()))
		if err != nil {
			continue
		}
		var run WorkflowRun
		if err := json.Unmarshal(data, &run); err != nil {
			continue
		}
		s.runs[run.ID] = &run
		loaded++
	}
	if loaded > 0 {
		log.Printf("[workflow] loaded %d persisted workflow runs", loaded)
	}
}

func generateID(prefix string) string {
	b := make([]byte, 8)
	rand.Read(b)
	return prefix + hex.EncodeToString(b)
}
