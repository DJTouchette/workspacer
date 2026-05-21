// Package template manages reusable task templates for Claude Code agents.
package template

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

// Template defines a reusable task configuration.
type Template struct {
	ID            string    `json:"id"`
	Name          string    `json:"name"`
	Description   string    `json:"description"`
	Prompt        string    `json:"prompt"`
	Autonomy      string    `json:"autonomy"`
	BudgetDollars float64   `json:"budgetDollars"`
	ProjectPath   string    `json:"projectPath,omitempty"` // empty = global
	IsBuiltIn     bool      `json:"isBuiltIn"`
	CreatedAt     time.Time `json:"createdAt"`
}

// Store manages template persistence and built-in templates.
type Store struct {
	mu        sync.RWMutex
	templates map[string]*Template
	dataDir   string
}

// builtInTemplates are always available and cannot be modified or deleted.
var builtInTemplates = []*Template{
	{
		ID:            "fix-lint",
		Name:          "Fix Lint",
		Description:   "Fix all lint errors and warnings",
		Prompt:        "Fix all lint errors and warnings",
		Autonomy:      "semi",
		BudgetDollars: 1,
		IsBuiltIn:     true,
	},
	{
		ID:            "add-tests",
		Name:          "Add Tests",
		Description:   "Add comprehensive tests for any untested files",
		Prompt:        "Add comprehensive tests for any untested files",
		Autonomy:      "semi",
		BudgetDollars: 2,
		IsBuiltIn:     true,
	},
	{
		ID:            "update-deps",
		Name:          "Update Dependencies",
		Description:   "Update all dependencies to latest compatible versions",
		Prompt:        "Update all dependencies to latest compatible versions",
		Autonomy:      "semi",
		BudgetDollars: 1,
		IsBuiltIn:     true,
	},
	{
		ID:            "code-review",
		Name:          "Code Review",
		Description:   "Review the codebase for bugs, security issues, and improvements",
		Prompt:        "Review the codebase for bugs, security issues, and improvements",
		Autonomy:      "manual",
		BudgetDollars: 2,
		IsBuiltIn:     true,
	},
	{
		ID:            "refactor",
		Name:          "Refactor",
		Description:   "Refactor for clarity and maintainability without changing behavior",
		Prompt:        "Refactor for clarity and maintainability without changing behavior",
		Autonomy:      "semi",
		BudgetDollars: 3,
		IsBuiltIn:     true,
	},
	{
		ID:            "docs",
		Name:          "Documentation",
		Description:   "Add or update documentation for public APIs and key modules",
		Prompt:        "Add or update documentation for public APIs and key modules",
		Autonomy:      "full",
		BudgetDollars: 1,
		IsBuiltIn:     true,
	},
}

// NewStore creates a template store, loading persisted templates and seeding built-ins.
func NewStore(dataDir string) *Store {
	s := &Store{
		templates: make(map[string]*Template),
		dataDir:   dataDir,
	}

	// Seed built-in templates
	for _, t := range builtInTemplates {
		cp := *t
		s.templates[cp.ID] = &cp
	}

	// Load user templates from disk
	if dataDir != "" {
		dir := filepath.Join(dataDir, "templates")
		os.MkdirAll(dir, 0755)
		s.loadTemplates(dir)
	}

	return s
}

// List returns all global templates plus project-specific ones if projectPath is given.
func (s *Store) List(projectPath string) []Template {
	s.mu.RLock()
	defer s.mu.RUnlock()

	var out []Template
	for _, t := range s.templates {
		if t.ProjectPath == "" || t.ProjectPath == projectPath {
			out = append(out, *t)
		}
	}
	return out
}

// Get returns a template by ID, or nil if not found.
func (s *Store) Get(id string) *Template {
	s.mu.RLock()
	defer s.mu.RUnlock()

	t := s.templates[id]
	if t == nil {
		return nil
	}
	cp := *t
	return &cp
}

// Create adds a new user template, generating an ID and setting CreatedAt.
func (s *Store) Create(t Template) Template {
	s.mu.Lock()
	defer s.mu.Unlock()

	t.ID = generateID()
	t.CreatedAt = time.Now()
	t.IsBuiltIn = false

	cp := t
	s.templates[t.ID] = &cp
	s.saveTemplate(&cp)

	return t
}

// Update modifies an existing user template. Returns an error if the template
// is built-in or does not exist.
func (s *Store) Update(id string, t Template) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	existing := s.templates[id]
	if existing == nil {
		return fmt.Errorf("template %q not found", id)
	}
	if existing.IsBuiltIn {
		return fmt.Errorf("cannot modify built-in template %q", id)
	}

	t.ID = id
	t.IsBuiltIn = false
	t.CreatedAt = existing.CreatedAt

	cp := t
	s.templates[id] = &cp
	s.saveTemplate(&cp)

	return nil
}

// Delete removes a user template. Returns an error if the template is built-in
// or does not exist.
func (s *Store) Delete(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	existing := s.templates[id]
	if existing == nil {
		return fmt.Errorf("template %q not found", id)
	}
	if existing.IsBuiltIn {
		return fmt.Errorf("cannot delete built-in template %q", id)
	}

	delete(s.templates, id)

	// Remove from disk
	if s.dataDir != "" {
		path := filepath.Join(s.dataDir, "templates", id+".json")
		os.Remove(path)
	}

	return nil
}

// ── Persistence ──

func (s *Store) saveTemplate(t *Template) {
	if s.dataDir == "" || t.IsBuiltIn {
		return
	}
	path := filepath.Join(s.dataDir, "templates", t.ID+".json")
	data, err := json.Marshal(t)
	if err != nil {
		log.Printf("[template] save error id=%s: %v", t.ID, err)
		return
	}
	if err := os.WriteFile(path, data, 0644); err != nil {
		log.Printf("[template] write error id=%s: %v", t.ID, err)
	}
}

func (s *Store) loadTemplates(dir string) {
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
		var t Template
		if err := json.Unmarshal(data, &t); err != nil {
			continue
		}
		// Don't overwrite built-in templates from disk
		if _, exists := s.templates[t.ID]; exists && s.templates[t.ID].IsBuiltIn {
			continue
		}
		s.templates[t.ID] = &t
		loaded++
	}
	if loaded > 0 {
		log.Printf("[template] loaded %d persisted templates", loaded)
	}
}

func generateID() string {
	b := make([]byte, 8)
	rand.Read(b)
	return "tpl-" + hex.EncodeToString(b)
}
