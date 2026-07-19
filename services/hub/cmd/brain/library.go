package main

// The library — reusable prompts/skills/agents as markdown files with YAML
// frontmatter. A Go port of libraryService.ts (minus the renderer watch/notify,
// which belongs to the streaming phase). Sources:
//
//	global:  <configDir>/library/*.md
//	project: <cwd>/.workspacer/library/*.md
//	claude:  <cwd>/.claude/skills/<id>/SKILL.md, <cwd>/.claude/agents/<id>.md
//
// Items merge with project winning over global on id collision; claude items are
// namespaced separately. Filenames use slugLibrary so they match the app.

import (
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	yaml "gopkg.in/yaml.v3"
)

type mcpConfig struct {
	Type    string            `json:"type,omitempty" yaml:"type,omitempty"`
	Command string            `json:"command,omitempty" yaml:"command,omitempty"`
	Args    []string          `json:"args,omitempty" yaml:"args,omitempty"`
	Env     map[string]string `json:"env,omitempty" yaml:"env,omitempty"`
	URL     string            `json:"url,omitempty" yaml:"url,omitempty"`
	Headers map[string]string `json:"headers,omitempty" yaml:"headers,omitempty"`
}

type libraryItem struct {
	ID          string     `json:"id"`
	Scope       string     `json:"scope"`
	Title       string     `json:"title"`
	Kind        string     `json:"kind"`
	Description string     `json:"description,omitempty"`
	Tags        []string   `json:"tags,omitempty"`
	Action      string     `json:"action,omitempty"`
	Mcp         *mcpConfig `json:"mcp,omitempty"`
	Body        string     `json:"body"`
	Path        string     `json:"path"`
}

func libraryGlobalDir() string            { return filepath.Join(configDir(), "library") }
func libraryProjectDir(cwd string) string { return filepath.Join(cwd, ".workspacer", "library") }
func claudeSkillsDir(cwd string) string   { return filepath.Join(cwd, ".claude", "skills") }
func claudeAgentsDir(cwd string) string   { return filepath.Join(cwd, ".claude", "agents") }
func claudeCommandsDir(cwd string) string { return filepath.Join(cwd, ".claude", "commands") }

var (
	reFrontmatter  = regexp.MustCompile(`(?s)^---\r?\n(.*?)\r?\n---\r?\n?(.*)$`)
	reLeadingBlank = regexp.MustCompile(`^\s*\n`)
)

// parseFrontmatter splits a markdown file into its YAML frontmatter map + body.
func parseFrontmatter(raw string) (map[string]any, string) {
	if m := reFrontmatter.FindStringSubmatch(raw); m != nil {
		var data map[string]any
		if err := yaml.Unmarshal([]byte(m[1]), &data); err == nil {
			if data == nil {
				data = map[string]any{}
			}
			return data, m[2]
		}
		// malformed frontmatter — fall through, treat whole file as body
	}
	return map[string]any{}, raw
}

func cleanMcp(c *mcpConfig) *mcpConfig {
	if c == nil {
		return nil
	}
	out := &mcpConfig{Type: c.Type}
	out.Command = strings.TrimSpace(c.Command)
	if len(c.Args) > 0 {
		out.Args = c.Args
	}
	if len(c.Env) > 0 {
		out.Env = c.Env
	}
	out.URL = strings.TrimSpace(c.URL)
	if len(c.Headers) > 0 {
		out.Headers = c.Headers
	}
	return out
}

func toMcp(v any) *mcpConfig {
	m, ok := v.(map[string]any)
	if !ok {
		return nil
	}
	b, err := yaml.Marshal(m)
	if err != nil {
		return nil
	}
	var c mcpConfig
	if yaml.Unmarshal(b, &c) != nil {
		return nil
	}
	return cleanMcp(&c)
}

// libFrontmatter is the workspacer-format frontmatter; struct field order is the
// emitted YAML order (title, kind, …), matching libraryService.serialize.
type libFrontmatter struct {
	Title       string     `yaml:"title"`
	Kind        string     `yaml:"kind"`
	Description string     `yaml:"description,omitempty"`
	Tags        []string   `yaml:"tags,omitempty"`
	Action      string     `yaml:"action,omitempty"`
	Mcp         *mcpConfig `yaml:"mcp,omitempty"`
}

func serializeItem(it *libraryItem) string {
	fm := libFrontmatter{Title: it.Title, Kind: it.Kind, Description: it.Description, Tags: it.Tags, Action: it.Action}
	if it.Kind == "mcp" {
		fm.Mcp = cleanMcp(it.Mcp)
	}
	head := strings.TrimRight(marshalYAML(fm), "\n")
	body := strings.TrimRight(it.Body, " \t\r\n\v\f")
	return "---\n" + head + "\n---\n\n" + body + "\n"
}

// claudeFrontmatter emits name/description first, then any preserved keys
// (inline), matching libraryService.serializeClaude.
type claudeFrontmatter struct {
	Name        string         `yaml:"name"`
	Description string         `yaml:"description,omitempty"`
	Rest        map[string]any `yaml:",inline"`
}

func serializeClaude(existing map[string]any, title, description, body string) string {
	rest := map[string]any{}
	for k, v := range existing {
		if k == "name" || k == "description" {
			continue
		}
		rest[k] = v
	}
	fm := claudeFrontmatter{Name: title, Description: description, Rest: rest}
	head := strings.TrimRight(marshalYAML(fm), "\n")
	b := strings.TrimRight(body, " \t\r\n\v\f")
	return "---\n" + head + "\n---\n\n" + b + "\n"
}

func marshalYAML(v any) string {
	b, err := yaml.Marshal(v)
	if err != nil {
		return ""
	}
	return string(b)
}

func validKind(k any) string {
	if s, ok := k.(string); ok && (s == "skill" || s == "agent" || s == "mcp") {
		return s
	}
	return "prompt"
}

func validAction(a any) string {
	if s, ok := a.(string); ok && (s == "insert" || s == "spawn" || s == "copy") {
		return s
	}
	return ""
}

func readLibraryDir(dir, scope string) []libraryItem {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var items []libraryItem
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || !strings.HasSuffix(strings.ToLower(name), ".md") {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(dir, name))
		if err != nil {
			continue
		}
		data, body := parseFrontmatter(string(raw))
		id := slugLibrary(strings.TrimSuffix(strings.TrimSuffix(name, ".md"), ".MD"))
		kind := validKind(data["kind"])
		it := libraryItem{
			ID:          id,
			Scope:       scope,
			Title:       firstNonEmpty(str(data["title"]), id),
			Kind:        kind,
			Description: str(data["description"]),
			Tags:        toStringSlice(data["tags"]),
			Action:      validAction(data["action"]),
			Body:        reLeadingBlank.ReplaceAllString(body, ""),
			Path:        filepath.Join(dir, name),
		}
		if kind == "mcp" {
			it.Mcp = toMcp(data["mcp"])
		}
		items = append(items, it)
	}
	return items
}

func readClaudeItem(full, id, kind string) *libraryItem {
	raw, err := os.ReadFile(full)
	if err != nil {
		return nil
	}
	data, body := parseFrontmatter(string(raw))
	return &libraryItem{
		ID:          id,
		Scope:       "claude",
		Title:       firstNonEmpty(str(data["name"]), id),
		Kind:        kind,
		Description: str(data["description"]),
		Body:        reLeadingBlank.ReplaceAllString(body, ""),
		Path:        full,
	}
}

func readClaudeItems(cwd string) []libraryItem {
	var items []libraryItem
	if entries, err := os.ReadDir(claudeSkillsDir(cwd)); err == nil {
		for _, e := range entries {
			if !e.IsDir() {
				continue
			}
			if it := readClaudeItem(filepath.Join(claudeSkillsDir(cwd), e.Name(), "SKILL.md"), slugLibrary(e.Name()), "skill"); it != nil {
				items = append(items, *it)
			}
		}
	}
	if entries, err := os.ReadDir(claudeAgentsDir(cwd)); err == nil {
		for _, e := range entries {
			name := e.Name()
			if e.IsDir() || !strings.HasSuffix(strings.ToLower(name), ".md") {
				continue
			}
			if it := readClaudeItem(filepath.Join(claudeAgentsDir(cwd), name), slugLibrary(strings.TrimSuffix(name, ".md")), "agent"); it != nil {
				items = append(items, *it)
			}
		}
	}
	// Custom slash commands: flat .md files. Their frontmatter carries no `name`
	// (the filename is the command), so readClaudeItem falls back to the id for
	// the title — exactly what the composer's "/" picker shows after the "/".
	if entries, err := os.ReadDir(claudeCommandsDir(cwd)); err == nil {
		for _, e := range entries {
			name := e.Name()
			if e.IsDir() || !strings.HasSuffix(strings.ToLower(name), ".md") {
				continue
			}
			if it := readClaudeItem(filepath.Join(claudeCommandsDir(cwd), name), slugLibrary(strings.TrimSuffix(name, ".md")), "command"); it != nil {
				items = append(items, *it)
			}
		}
	}
	return items
}

// listLibrary merges global + project (project wins on id) + claude (namespaced),
// sorted by title. Seeds the global dir with starter items on first use.
func listLibrary(cwd string) []libraryItem {
	seedLibraryIfEmpty()
	byID := map[string]libraryItem{}
	order := []string{}
	put := func(key string, it libraryItem) {
		if _, ok := byID[key]; !ok {
			order = append(order, key)
		}
		byID[key] = it
	}
	for _, it := range readLibraryDir(libraryGlobalDir(), "global") {
		put(it.ID, it)
	}
	if cwd != "" {
		for _, it := range readLibraryDir(libraryProjectDir(cwd), "project") {
			put(it.ID, it)
		}
		for _, it := range readClaudeItems(cwd) {
			put("claude:"+it.Kind+":"+it.ID, it)
		}
	}
	out := make([]libraryItem, 0, len(byID))
	for _, k := range order {
		out = append(out, byID[k])
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Title < out[j].Title })
	return out
}

// libraryInput is the save payload (matches the app's library.save params).
type libraryInput struct {
	Scope       string     `json:"scope"`
	ID          string     `json:"id"`
	Title       string     `json:"title"`
	Kind        string     `json:"kind"`
	Description string     `json:"description"`
	Tags        []string   `json:"tags"`
	Action      string     `json:"action"`
	Mcp         *mcpConfig `json:"mcp"`
	Body        string     `json:"body"`
	Cwd         string     `json:"cwd"`
}

func saveLibrary(in libraryInput) (*libraryItem, error) {
	if in.Scope == "claude" {
		return saveLibraryClaude(in)
	}
	dir := libraryGlobalDir()
	if in.Scope == "project" {
		dir = libraryProjectDir(firstNonEmpty(in.Cwd, mustCwd()))
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return nil, err
	}
	id := slugLibrary(firstNonEmpty(in.ID, in.Title))
	full := filepath.Join(dir, id+".md")
	it := &libraryItem{
		ID: id, Scope: in.Scope, Title: in.Title, Kind: in.Kind,
		Description: in.Description, Tags: in.Tags, Action: in.Action,
		Body: in.Body, Path: full,
	}
	if in.Kind == "mcp" {
		it.Mcp = cleanMcp(in.Mcp)
	}
	if err := os.WriteFile(full, []byte(serializeItem(it)), 0o644); err != nil {
		return nil, err
	}
	return it, nil
}

func saveLibraryClaude(in libraryInput) (*libraryItem, error) {
	cwd := firstNonEmpty(in.Cwd, mustCwd())
	kind := "skill"
	if in.Kind == "agent" {
		kind = "agent"
	} else if in.Kind == "command" {
		kind = "command"
	}
	id := slugLibrary(firstNonEmpty(in.ID, in.Title))
	var full string
	switch kind {
	case "skill":
		full = filepath.Join(claudeSkillsDir(cwd), id, "SKILL.md")
	case "command":
		full = filepath.Join(claudeCommandsDir(cwd), id+".md")
	default:
		full = filepath.Join(claudeAgentsDir(cwd), id+".md")
	}
	if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
		return nil, err
	}
	existing := map[string]any{}
	if raw, err := os.ReadFile(full); err == nil {
		existing, _ = parseFrontmatter(string(raw))
	}
	if err := os.WriteFile(full, []byte(serializeClaude(existing, in.Title, in.Description, in.Body)), 0o644); err != nil {
		return nil, err
	}
	return &libraryItem{ID: id, Scope: "claude", Title: in.Title, Kind: kind, Description: in.Description, Body: in.Body, Path: full}, nil
}

func removeLibrary(scope, id, cwd, kind string) {
	if scope == "claude" {
		root := firstNonEmpty(cwd, mustCwd())
		switch kind {
		case "agent":
			_ = os.Remove(filepath.Join(claudeAgentsDir(root), slugLibrary(id)+".md"))
		case "command":
			_ = os.Remove(filepath.Join(claudeCommandsDir(root), slugLibrary(id)+".md"))
		default:
			_ = os.RemoveAll(filepath.Join(claudeSkillsDir(root), slugLibrary(id)))
		}
		return
	}
	dir := libraryGlobalDir()
	if scope == "project" {
		dir = libraryProjectDir(firstNonEmpty(cwd, mustCwd()))
	}
	_ = os.Remove(filepath.Join(dir, slugLibrary(id)+".md"))
}

// seedLibraryIfEmpty writes starter items to the global dir on first use, the
// same three the app ships (seedGlobalIfEmpty). Best-effort and idempotent: it
// no-ops once any .md exists.
func seedLibraryIfEmpty() {
	dir := libraryGlobalDir()
	if entries, err := os.ReadDir(dir); err == nil {
		for _, e := range entries {
			if strings.HasSuffix(strings.ToLower(e.Name()), ".md") {
				return
			}
		}
	}
	if os.MkdirAll(dir, 0o755) != nil {
		return
	}
	seeds := []libraryItem{
		{
			Title: "Summarize & plan", Kind: "prompt", Action: "insert",
			Description: "Have the agent summarize the codebase area and propose a plan.",
			Tags:        []string{"planning"},
			Body:        "Summarize how `{{cwd}}` is structured at a high level, then propose a step-by-step plan for: {{?What do you want to do?}}\n\nList the files you would touch and call out the riskiest step before writing any code.",
		},
		{
			Title: "Careful refactor (skill)", Kind: "skill", Action: "insert",
			Description: "A disciplined refactor workflow: small steps, tests between each.",
			Tags:        []string{"refactor", "tests"},
			Body: strings.Join([]string{
				"When refactoring, follow this workflow strictly:",
				"",
				"1. First, identify the smallest safe unit to change and state it.",
				"2. Make ONE change, then run the relevant tests/build.",
				"3. Only proceed to the next change once green. Never batch unrelated edits.",
				"4. Preserve public behavior; if a signature must change, note every caller.",
				"5. At the end, summarize what changed and what you verified.",
				"",
				"Begin by mapping the change surface for: {{?Target to refactor?}}",
			}, "\n"),
		},
		{
			Title: "Context7 (MCP)", Kind: "mcp",
			Description: "Example MCP server — up-to-date library docs. Select it at spawn to expose its tools.",
			Tags:        []string{"docs", "example"},
			Mcp:         &mcpConfig{Type: "stdio", Command: "npx", Args: []string{"-y", "@upstash/context7-mcp"}},
			Body:        "An example MCP server entry. Edit the command/args (or switch to an http URL), then pick it in the spawn dialog to load it for a session.",
		},
		{
			Title: "Make a workspacer plugin (skill)", Kind: "skill", Action: "insert",
			Description: "Scaffold and implement a workspacer plugin (webview or sidecar) that talks the hub bus.",
			Tags:        []string{"plugin", "dev"},
			Body: strings.Join([]string{
				`Build a workspacer plugin that talks the hub bus. Pick one kind:`,
				``,
				`- webview: a pane served from ui/index.html; may use ${agentCwd}-scoped capabilities.`,
				`- sidecar: a zero-dependency Node process (server.js); Node >=22 built-ins only.`,
				``,
				`1) plugin.json - apiVersion MUST be exactly "1"; id is "owner.name".`,
				``,
				`Sidecar:`,
				`{`,
				`  "id": "you.my-plugin", "name": "My Plugin", "apiVersion": "1",`,
				`  "server": { "command": "node", "args": ["server.js"], "port": 9300, "health": "/health" },`,
				`  "capabilities": ["agents.sendMessage", "notifications.post"],`,
				`  "consumes": ["agent.state_changed"]`,
				`}`,
				``,
				`Webview (omit server; set ui + a pane):`,
				`{`,
				`  "id": "you.my-plugin", "name": "My Plugin", "apiVersion": "1", "ui": "ui",`,
				`  "panes": [{ "type": "you.my-plugin", "title": "My Plugin", "scope": "both", "path": "/" }],`,
				`  "capabilities": ["agents.list"], "consumes": ["agent.state_changed"]`,
				`}`,
				``,
				`Rules (fail-closed; undeclared is silently denied):`,
				`- Only call methods in capabilities, publish types in emits, receive types in consumes.`,
				`- fs.* and search.project need object form: { "method": "fs.read", "paths": ["${pluginDir}"] }.`,
				`  ${agentCwd} resolves only for per-pane webview tokens; a sidecar watches files locally via Node fs.`,
				`- Never hand-write .bus-token/.settings.json/.install-source/.disabled; gitignore them.`,
				``,
				`2) Talk to the bus.`,
				`Webview: the host auto-injects window.workspacer (no bus boilerplate). Use:`,
				`  await workspacer.ready`,
				`  workspacer.on(type, (data) => {})     receives only your declared consumes types`,
				`  await workspacer.call(method, params)     only your declared capabilities`,
				`  workspacer.publish(type, data)`,
				`  workspacer.settings                      live; workspacer.onSettings(cb) for changes`,
				`Sidecar: connect to ws://127.0.0.1:7895/bus?token=<t> and speak JSON frames:`,
				`- {op:"subscribe", topics:[...]}          (topics allow ns.* and *)`,
				`- {op:"call", id, method, params}  ->  {op:"result", id, result} or {op:"error", id, error}`,
				`- {op:"publish", event:{type, source, data}}    inbound: {op:"event", event}`,
				`Token: a sidecar reads env HUB_TOKEN; a webview needs no token (the SDK is wired).`,
				``,
				`3) Develop with hot-reload:`,
				`    workspacer plugin dev <plugin-dir>`,
				`boots the backend against just this plugin and reloads it on every save.`,
				``,
				`Common capabilities: agents.list, agents.sendMessage, notifications.post (params in`,
				`apps/desktop/src/main/services/hubCapabilities.ts). Common events: agent.state_changed`,
				`{sessionId,mode,cwd}, agent.snapshot, workflow.completed, fs.changed (after fs.watch).`,
				``,
				`Full guide: the "build a plugin" page on the landing site (build-plugin.html and build-plugin.md).`,
				`Working examples: the workspacer-plugins catalog (test-on-save = sidecar, cost-hud = webview).`,
				``,
				`Tell me the plugin name and what it should do, and I will scaffold and implement it: {{?What should the plugin do?}}`,
			}, "\n"),
		},
	}
	names := []string{"summarize-and-plan.md", "careful-refactor.md", "context7-mcp.md", "make-workspacer-plugin.md"}
	for i := range seeds {
		_ = os.WriteFile(filepath.Join(dir, names[i]), []byte(serializeItem(&seeds[i])), 0o644)
	}
}

func mustCwd() string {
	cwd, err := os.Getwd()
	if err != nil {
		return ""
	}
	return cwd
}

func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		if v != "" {
			return v
		}
	}
	return ""
}
