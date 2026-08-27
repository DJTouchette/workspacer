package main

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

var sessionFacadeTokenMu sync.Mutex

const sessionFacadeTokenLabelPrefix = "session:"

type sessionFacade struct {
	SessionID    string
	BaseURL      string
	URL          string
	Token        string
	Instructions string
}

func (p spawnParams) wantsFacade() bool {
	return p.Supervisor || p.MCPFacade || strings.TrimSpace(p.ToolScope) != ""
}

func (p spawnParams) facadeScope() (authtoken.Scope, error) {
	if p.Supervisor {
		return authtoken.ScopeOperator, nil
	}
	if strings.TrimSpace(p.ToolScope) == "" {
		return authtoken.ScopeOperator, nil
	}
	scope, err := authtoken.ParseScope(p.ToolScope)
	if err != nil {
		return "", fmt.Errorf("invalid toolScope %q: %w", p.ToolScope, err)
	}
	if scope == authtoken.ScopeProvider {
		return "", fmt.Errorf("invalid toolScope %q: provider scope is only for capability processes", p.ToolScope)
	}
	return scope, nil
}

func (r *registry) buildSessionFacade(sessionID string, p spawnParams) (*sessionFacade, error) {
	if !p.wantsFacade() {
		return nil, nil
	}
	baseURL := strings.TrimSpace(r.mcpFacadeURL)
	if baseURL == "" {
		return nil, fmt.Errorf("workspacer MCP facade requested for session %s, but brain was started without --mcp-facade", sessionID)
	}
	if err := validateSessionConfigName(sessionID); err != nil {
		return nil, err
	}
	scope, err := p.facadeScope()
	if err != nil {
		return nil, err
	}

	role := ""
	yoloAllowed := false
	var profilesAllowed []string
	switch {
	case p.Manager:
		role = "manager"
		yoloAllowed = r.managerFullAccessFromConfig()
		profilesAllowed = localProfileIDs()
	case p.Supervisor:
		role = "supervisor"
		yoloAllowed = r.supervisorFullAccessFromConfig()
	}

	rec, err := mintSessionFacadeToken(sessionID, scope, p.PluginTools, profilesAllowed, yoloAllowed, role)
	if err != nil {
		return nil, err
	}
	u, err := facadeURLWithToken(baseURL, rec.Token)
	if err != nil {
		return nil, err
	}
	return &sessionFacade{
		SessionID:    sessionID,
		BaseURL:      baseURL,
		URL:          u,
		Token:        rec.Token,
		Instructions: sessionFacadeInstructions(sessionID, p),
	}, nil
}

func mintSessionFacadeToken(sessionID string, scope authtoken.Scope, pluginsAllowed []string, profilesAllowed []string, yoloAllowed bool, role string) (authtoken.Record, error) {
	token, err := randomFacadeToken()
	if err != nil {
		return authtoken.Record{}, err
	}
	rec := authtoken.Record{
		Token:           token,
		Scope:           scope,
		Label:           sessionFacadeTokenLabelPrefix + sessionID,
		Created:         time.Now().UTC().Truncate(time.Second),
		Plugins:         cleanStringList(pluginsAllowed),
		ProfilesAllowed: cleanStringList(profilesAllowed),
		YoloAllowed:     yoloAllowed,
		Role:            strings.TrimSpace(role),
	}

	path := authtoken.DefaultPath()
	sessionFacadeTokenMu.Lock()
	defer sessionFacadeTokenMu.Unlock()
	records, err := authtoken.Load(path)
	if err != nil {
		return authtoken.Record{}, err
	}
	label := rec.Label
	next := make([]authtoken.Record, 0, len(records)+1)
	for _, existing := range records {
		if existing.Label != label {
			next = append(next, existing)
		}
	}
	next = append(next, rec)
	if err := authtoken.Save(path, next); err != nil {
		return authtoken.Record{}, err
	}
	return rec, nil
}

func randomFacadeToken() (string, error) {
	var raw [24]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(raw[:]), nil
}

func facadeURLWithToken(rawURL, token string) (string, error) {
	u, err := url.Parse(rawURL)
	if err != nil {
		return "", err
	}
	q := u.Query()
	q.Set("t", token)
	u.RawQuery = q.Encode()
	return u.String(), nil
}

func (f *sessionFacade) claudeArgs() ([]string, error) {
	path, err := writeClaudeFacadeMCPConfig(f.SessionID, f.BaseURL, f.Token)
	if err != nil {
		return nil, err
	}
	return []string{
		"--mcp-config", path,
		"--allowedTools", "mcp__workspacer",
		"--append-system-prompt", f.Instructions,
	}, nil
}

type claudeMCPConfig struct {
	MCPServers map[string]claudeMCPServer `json:"mcpServers"`
}

type claudeMCPServer struct {
	Type    string            `json:"type"`
	URL     string            `json:"url"`
	Headers map[string]string `json:"headers,omitempty"`
}

func writeClaudeFacadeMCPConfig(sessionID string, facadeURL string, token string) (string, error) {
	if err := validateSessionConfigName(sessionID); err != nil {
		return "", err
	}
	baseDir := configDir()
	if strings.TrimSpace(baseDir) == "" {
		return "", fmt.Errorf("could not resolve workspacer config dir")
	}
	dir := filepath.Join(baseDir, "session-mcp")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", err
	}
	path := filepath.Join(dir, sessionID+".json")
	body, err := json.MarshalIndent(claudeMCPConfig{
		MCPServers: map[string]claudeMCPServer{
			"workspacer": {
				Type: "http",
				URL:  strings.TrimSpace(facadeURL),
				Headers: map[string]string{
					"Authorization": "Bearer " + token,
				},
			},
		},
	}, "", "  ")
	if err != nil {
		return "", err
	}
	body = append(body, '\n')
	if err := writeFileAtomic0600(path, body); err != nil {
		return "", err
	}
	return path, nil
}

func validateSessionConfigName(sessionID string) error {
	if strings.TrimSpace(sessionID) == "" {
		return fmt.Errorf("empty session id")
	}
	if strings.ContainsAny(sessionID, `/\`) || filepath.Base(sessionID) != sessionID || sessionID == "." || sessionID == ".." {
		return fmt.Errorf("invalid session id %q", sessionID)
	}
	return nil
}

func writeFileAtomic0600(path string, data []byte) error {
	dir := filepath.Dir(path)
	tmp, err := os.CreateTemp(dir, ".tmp-")
	if err != nil {
		return err
	}
	tmpPath := tmp.Name()
	cleanup := true
	defer func() {
		if cleanup {
			_ = os.Remove(tmpPath)
		}
	}()
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(tmpPath, path); err != nil {
		return err
	}
	cleanup = false
	return nil
}

func sessionFacadeInstructions(sessionID string, p spawnParams) string {
	scope := strings.TrimSpace(p.ToolScope)
	if p.Supervisor {
		scope = string(authtoken.ScopeOperator)
	}
	if scope == "" {
		scope = string(authtoken.ScopeOperator)
	}
	parts := []string{
		fmt.Sprintf("You are running inside Workspacer session %s with access to the local workspacer MCP facade.", sessionID),
		fmt.Sprintf("Use the workspacer MCP tools when they are relevant to the task. Your tool scope for this session is %s.", scope),
	}
	if scope == string(authtoken.ScopeView) {
		parts = append(parts, "Treat workspacer tools as read-only unless another tool separately permits a change.")
	}
	if p.Manager {
		parts = append(parts, "You are the session manager; use workspacer tools to coordinate child sessions and report their status when needed.")
	}
	if p.Supervisor {
		parts = append(parts, "You are the session supervisor; use workspacer tools to inspect and manage the session only when it serves the user's request.")
	}
	return strings.Join(parts, "\n")
}

func defaultMCPFacadeURL() string {
	return "http://127.0.0.1:7897/mcp"
}

func cleanStringList(values []string) []string {
	seen := make(map[string]bool, len(values))
	out := make([]string, 0, len(values))
	for _, raw := range values {
		v := strings.TrimSpace(raw)
		if v == "" || seen[v] {
			continue
		}
		seen[v] = true
		out = append(out, v)
	}
	return out
}

func localProfileIDs() []string {
	profiles := loadProfiles()
	out := make([]string, 0, len(profiles))
	for _, p := range profiles {
		if id := strings.TrimSpace(p.ID); id != "" {
			out = append(out, id)
		}
	}
	return cleanStringList(out)
}

func (r *registry) managerFullAccessFromConfig() bool {
	cfg := r.cfg.get()
	if configBool(cfg, "agents", "fleetFullAccess") {
		return true
	}
	projects, ok := cfg["projects"].(map[string]any)
	if !ok {
		return false
	}
	for _, raw := range projects {
		project, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		if yolo, ok := project["yolo"].(bool); ok && yolo {
			return true
		}
	}
	return false
}

func (r *registry) supervisorFullAccessFromConfig() bool {
	return configBool(r.cfg.get(), "supervisor", "fullAccess")
}

func configBool(cfg map[string]any, section string, key string) bool {
	rawSection, ok := cfg[section].(map[string]any)
	if !ok {
		return false
	}
	v, ok := rawSection[key].(bool)
	return ok && v
}
