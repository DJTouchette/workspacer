package main

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"sort"
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
	LibraryMCP   []namedMCPServer
}

type namedMCPServer struct {
	ID     string
	Config mcpConfig
}

func (p spawnParams) wantsFacade() bool {
	return true
}

func (p spawnParams) facadeScope() (authtoken.Scope, error) {
	return authtoken.ScopeOperator, nil
}

func (r *registry) buildSessionFacade(sessionID string, p spawnParams) (*sessionFacade, error) {
	if !p.wantsFacade() {
		return nil, nil
	}
	baseURL := strings.TrimSpace(r.mcpFacadeURL)
	if baseURL == "" {
		// Only the process that owns and health-checked a facade may advertise
		// one. A bare brain (including an older `workspacer serve`) must never
		// manufacture a loopback URL that has no listener behind it.
		return nil, nil
	}
	if err := validateSessionConfigName(sessionID); err != nil {
		return nil, err
	}
	scope, err := p.facadeScope()
	if err != nil {
		return nil, err
	}

	role := ""
	switch {
	case p.Manager:
		role = "manager"
	}

	rec, err := mintSessionFacadeToken(sessionID, scope, []string{"*"}, nil, false, role)
	if err != nil {
		return nil, err
	}
	u, err := facadeURLWithToken(baseURL, rec.Token)
	if err != nil {
		_ = revokeSessionFacadeToken(sessionID)
		return nil, err
	}
	mcpIDs := append([]string{}, p.MCPItemIDs...)
	if prof := getProfile(p.ProfileID); prof != nil && prof.Provider == "" {
		mcpIDs = append(mcpIDs, prof.MCPItemIDs...)
	}
	return &sessionFacade{
		SessionID:    sessionID,
		BaseURL:      baseURL,
		URL:          u,
		Token:        rec.Token,
		Instructions: sessionFacadeInstructions(sessionID, p),
		LibraryMCP:   selectedMCPServers(p.Cwd, mcpIDs),
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

func (f *sessionFacade) claudeArgs(extraInstructions string) ([]string, error) {
	path, err := writeClaudeFacadeMCPConfig(f.SessionID, f.BaseURL, f.Token, f.LibraryMCP)
	if err != nil {
		return nil, err
	}
	instructions := f.Instructions
	if strings.TrimSpace(extraInstructions) != "" {
		instructions += "\n\n" + extraInstructions
	}
	toolNames := []string{"mcp__workspacer"}
	for _, server := range f.LibraryMCP {
		toolNames = append(toolNames, "mcp__"+server.ID)
	}
	args := []string{
		"--mcp-config", path,
		"--allowedTools", strings.Join(toolNames, ","),
		"--append-system-prompt", instructions,
	}
	if len(f.LibraryMCP) > 0 {
		args = append(args, "--strict-mcp-config")
	}
	return args, nil
}

type claudeMCPConfig struct {
	MCPServers map[string]claudeMCPServer `json:"mcpServers"`
}

type claudeMCPServer struct {
	Type    string            `json:"type"`
	URL     string            `json:"url,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
	Command string            `json:"command,omitempty"`
	Args    []string          `json:"args,omitempty"`
	Env     map[string]string `json:"env,omitempty"`
}

func writeClaudeFacadeMCPConfig(sessionID string, facadeURL string, token string, library []namedMCPServer) (string, error) {
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
	servers := map[string]claudeMCPServer{
		"workspacer": {
			Type: "http",
			URL:  strings.TrimSpace(facadeURL),
			Headers: map[string]string{
				"Authorization": "Bearer " + token,
			},
		},
	}
	for _, item := range library {
		if item.ID == "" || item.ID == "workspacer" {
			continue
		}
		servers[item.ID] = claudeMCPServer{
			Type: item.Config.Type, URL: item.Config.URL, Headers: item.Config.Headers,
			Command: item.Config.Command, Args: item.Config.Args, Env: item.Config.Env,
		}
	}
	body, err := json.MarshalIndent(claudeMCPConfig{MCPServers: servers}, "", "  ")
	if err != nil {
		return "", err
	}
	body = append(body, '\n')
	if err := writeFileAtomic0600(path, body); err != nil {
		return "", err
	}
	return path, nil
}

func selectedMCPServers(cwd string, ids []string) []namedMCPServer {
	wanted := map[string]bool{}
	for _, id := range ids {
		if id = strings.TrimSpace(id); id != "" {
			wanted[id] = true
		}
	}
	if len(wanted) == 0 {
		return nil
	}
	canonicalCwd, err := assertPathAllowed("agents.spawn", cwd, nil)
	if err != nil {
		return nil
	}
	guard := libraryFileGuardFor("agents.spawn", canonicalCwd)
	byID := map[string]libraryItem{}
	for _, item := range readLibraryDir(libraryGlobalDir(), "global", guard) {
		if wanted[item.ID] && item.Kind == "mcp" && item.Mcp != nil {
			byID[item.ID] = item
		}
	}
	for _, item := range readLibraryDir(libraryProjectDir(canonicalCwd), "project", guard) {
		if wanted[item.ID] && item.Kind == "mcp" && item.Mcp != nil {
			byID[item.ID] = item
		}
	}
	keys := make([]string, 0, len(byID))
	for id := range byID {
		keys = append(keys, id)
	}
	sort.Strings(keys)
	out := make([]namedMCPServer, 0, len(keys))
	for _, id := range keys {
		item := byID[id]
		out = append(out, namedMCPServer{ID: id, Config: *item.Mcp})
	}
	return out
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
	scope := string(authtoken.ScopeOperator)
	parts := []string{
		fmt.Sprintf("You are running inside Workspacer session %s with access to the local workspacer MCP facade.", sessionID),
		fmt.Sprintf("Use the workspacer MCP tools when they are relevant to the task. Your tool scope for this session is %s.", scope),
	}
	if !p.Manager {
		parts = append(parts, headlessAgentCollaborationInstructions)
	}
	if scope == string(authtoken.ScopeView) {
		parts = append(parts, "Treat workspacer tools as read-only unless another tool separately permits a change.")
	}
	if p.Manager {
		parts = append(parts, "You are the session manager; use workspacer tools to coordinate child sessions and report their status when needed.")
	}
	return strings.Join(parts, "\n")
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

// Legacy compatibility shim. Grant fields in config and tokens.json remain
// parseable for lossless upgrades, but supported agents always receive the
// ambient operator facade and these fields are never consulted or rewritten.
func (r *registry) reconcileManagerGrants(sessionID string) (bool, error) {
	_ = sessionID
	return false, nil
}
