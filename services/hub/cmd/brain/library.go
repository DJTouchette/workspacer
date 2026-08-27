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
	"context"
	"encoding/json"
	"fmt"
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
	// ResultSchema is a dispatch template's default structured-result contract
	// (kind "dispatch" only). Together with Body it is the WHOLE of what a
	// dispatch item carries — deliberately no spawn-argument fields, so a
	// template file cannot smuggle a toolScope/cwd/model/worktree; see
	// libraryService.ts's LibraryKind comment (the desktop twin).
	ResultSchema map[string]any `json:"resultSchema,omitempty"`
	// Origin is which root a claude-scoped item came from. Over the bus this is
	// always "project": libraryItemRoots confines every library file to the
	// caller's project plus the global store, so the user's ~/.claude and the
	// plugin roots the DESKTOP path also lists are unreachable here by
	// construction. The field ships anyway because the renderer keys, badges and
	// delete target all read it, and a claude item arriving without one would
	// route a later save/remove at the wrong root.
	Origin string `json:"origin,omitempty"`
	// Editable is false only for a plugin's files, which this side never
	// returns — see Origin. Emitted unconditionally so the shape matches
	// libraryService.ts's LibraryItem rather than differing by absence.
	Editable bool   `json:"editable"`
	Body     string `json:"body"`
	Path     string `json:"path"`
}

// claudeOriginProject is the only origin reachable over the bus (see
// libraryItem.Origin).
const claudeOriginProject = "project"

// assertWritableOrigin gates the origin a claude-scope WRITE or DELETE claims,
// the Go twin of libraryService.ts's function of the same name. A plugin's
// assets belong to the installed package: editing one is reverted by the next
// plugin update and deleting one corrupts the install, so it is refused rather
// than attempted. "user" is refused a step later, by the item-path guard, since
// ~/.claude is outside the roots this side may touch at all.
func assertWritableOrigin(origin string) error {
	if strings.HasPrefix(origin, "plugin:") {
		return fmt.Errorf("library: %s items are read-only — copy it into the project to edit it", origin)
	}
	return nil
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

// libraryFileGuard validates ONE DERIVED library file — not the caller's cwd —
// and returns the canonical path to open, or ok=false to skip it entirely.
//
// Guarding the cwd alone was not containment. library.list confined `cwd` and
// then handed the (allowed) directory to a walker that os.ReadFile'd every
// <dir>/<name>.md it found; library.remove confined `cwd` and then
// os.Remove/os.RemoveAll'd <cwd>/.claude/skills/<id>. Neither derived path was
// ever canonicalized, so ONE symlink planted inside an allowed root — a
// `.workspacer/library/pwn.md -> ~/.config/workspacer/remote-token`, or a
// `.claude/skills -> ~/.config/workspacer` directory link — read the bus
// credential out through library.list (fs.read of the identical symlink is
// refused) or deleted config.yaml through library.remove. Writing that symlink
// is an ordinary allowed fs.write into the root.
//
// BINDING DECISION 2 in one sentence: the path that is CHECKED must be the path
// that is OPENED. So every read and every unlink below goes through this.
type libraryFileGuard func(path string) (canonical string, ok bool)

// allowAnyLibraryFile is the identity guard, for the markdown-format unit tests
// that build their own temp trees and have no registry. It is deliberately not
// a default parameter value: a bus-reachable call site that forgot to pass a
// guard would not compile.
func allowAnyLibraryFile(path string) (string, bool) { return path, true }

// libraryItemRoots is the allow-list for DERIVED library files, and it is
// deliberately NARROWER than the roots the capability's `cwd` was checked
// against. Every file listLibrary reads and removeLibrary unlinks lives in
// exactly two places: the global store (<configDir>/library) and the project the
// caller named.
//
// library.list checks its cwd against the BROWSE roots — workspace roots plus
// the whole home tree — because the New Agent dialog lists the library of a
// directory no agent is running in yet. Handing those same roots to the PER-FILE
// guard turned the derived-path fix into an arbitrary home-directory reader: a
// `<cwd>/.workspacer/library/a.md -> ~/.ssh/id_rsa` symlink (the ordinary
// real-world form — git stores symlinks verbatim, so a clone carries them)
// canonicalized inside $HOME, passed the guard, and came back as an item Body,
// while fs.read of the identical path is refused. browseRoots exists for
// fs.listDir, which returns directory NAMES; library.list returns file BODIES.
//
// SAVE USES THIS LIST TOO. It did not, and that was a live divergence from the
// desktop twin: saveLibrary/saveLibraryClaude guarded their derived destination
// against r.workspaceRoots(ctx) — EVERY live agent cwd plus all three config
// stores — while hubCapabilities.ts guarded it against these item roots. So a
// `<projA>/.workspacer/library -> <projB>` directory symlink (an ordinary
// permitted fs.write, and the form a git clone carries verbatim) let one bus
// call with cwd=<projA> write attacker markdown into a SECOND project, and into
// <configDir>/sessions, on the copy that actually answers under
// DELEGATE_CATALOG_TO_BRAIN — while the desktop refused the identical call. The
// bus's own scoping makes it worse rather than better: capspec.PathParam
// ["library.save"] is "cwd", so a plugin granted paths:[<projA>] is authorized
// on the cwd alone and everything past it is the provider's job.
//
// It also left the brain disagreeing with ITSELF: save wrote items that its own
// remove (already on the item roots) then refused to delete.
func libraryItemRoots(canonicalCwd string) []string {
	roots := []string{libraryGlobalDir()}
	if canonicalCwd != "" {
		roots = append(roots, canonicalCwd)
	}
	return roots
}

// trimMDSuffix drops a case-insensitive ".md" extension, matching the regex
// replace of /\.md$/i the desktop does. TrimSuffix(name, ".md") alone leaves
// ".MD" and ".Md" on, so the two sides would mint different ids for the same
// file on the case-insensitive volumes where those names are ordinary.
func trimMDSuffix(name string) string {
	if len(name) >= 3 && strings.EqualFold(name[len(name)-3:], ".md") {
		return name[:len(name)-3]
	}
	return name
}

// assertPlainBasename is the claude-scope id gate, and the Go twin of
// libraryService.ts's function of the same name.
//
// A claude-scoped id is a REAL ON-DISK BASENAME, not a slug (see
// readClaudeItems), so it reaches filepath.Join unfiltered — which is a path
// injection point now that library.save / library.remove are bus-reachable: an
// id of "../../.." aimed saveLibraryClaude's write, and removeLibrary's
// os.RemoveAll, at whatever that composed to. Slugging is not the fix (it breaks
// every non-slug-stable name, which is the whole reason claude ids are
// basenames); requiring what a basename actually IS, is: one path segment,
// neither "." nor "..", no separator of either flavour — a backslash is only a
// separator on Windows, but a Windows-shaped id is not a legitimate item name on
// any platform.
func assertPlainBasename(id string) (string, bool) {
	if id == "" || id == "." || id == ".." {
		return "", false
	}
	if strings.ContainsAny(id, `/\`) || filepath.IsAbs(id) {
		return "", false
	}
	return id, true
}

// libraryItemDirs are the directories a library item file may actually LIVE in,
// composed from the canonical cwd and compared LEXICALLY (containsPath, not
// isWithin) against the already-canonical derived path.
//
// This is the second half of the derived-path gate and it exists because
// libraryItemRoots alone is only as narrow as the cwd the caller named. The cwd
// for library.list is checked against the BROWSE roots, so a caller may name
// $HOME itself — and then "the project the caller named" IS the whole home tree
// and the narrowing evaporates: a `$HOME/.workspacer/library/a.md ->
// $HOME/.ssh/id_rsa` symlink (the ordinary form, since git stores symlinks
// verbatim and a clone carries them) resolves inside the root and comes back as
// an item Body, while fs.read of the identical path is refused. Requiring the
// RESOLVED file to sit in a library directory says the thing the roots test was
// only approximating: a library item lives in a library directory.
//
// Lexical on purpose. Canonicalizing these would resolve a
// `<cwd>/.workspacer/library -> <projB>` link and hand the escape back.
func libraryItemDirs(canonicalCwd string) []string {
	dirs := []string{}
	// The global store is the one entry that must be RESOLVED: configDir()
	// itself is routinely a symlinked path (XDG_CONFIG_HOME on a linked volume,
	// /var -> /private/var on macOS), so a lexical comparison against it would
	// reject every global item.
	if gr, ok := canonicalRoot(libraryGlobalDir()); ok {
		dirs = append(dirs, gr)
	}
	if canonicalCwd != "" {
		dirs = append(dirs,
			libraryProjectDir(canonicalCwd),
			filepath.Join(canonicalCwd, ".claude"),
		)
	}
	return dirs
}

// assertLibraryItemPath is the whole derived-path gate for a library file: the
// same assertPathAllowed the fs.* handlers use, over libraryItemRoots, and then
// the item-directory requirement above. Returns the canonical path to open
// (BINDING DECISION 2).
func assertLibraryItemPath(capability, full, canonicalCwd string) (string, error) {
	canonical, err := assertPathAllowed(capability, full, libraryItemRoots(canonicalCwd))
	if err != nil {
		return "", err
	}
	for _, dir := range libraryItemDirs(canonicalCwd) {
		if containsPath(dir, canonical) {
			return canonical, nil
		}
	}
	// Same non-echoing message as every other refusal on this surface.
	return "", fmt.Errorf("%s: path is outside the allowed workspace (agent cwds + config stores)", capability)
}

// libraryFileGuardFor builds the real guard around assertLibraryItemPath.
// A refusal skips that one item rather than failing the whole call — a project
// with one poisoned symlink still lists its other prompts.
func libraryFileGuardFor(capability, canonicalCwd string) libraryFileGuard {
	return func(path string) (string, bool) {
		canonical, err := assertLibraryItemPath(capability, path, canonicalCwd)
		if err != nil {
			return "", false
		}
		return canonical, true
	}
}

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

// secretPlaceholder is what a stored MCP credential reads as once it leaves
// this process. The same literal as plugin settings' SecretPlaceholder
// (services/hub/internal/plugin/settings.go) and the desktop twin's
// SECRET_PLACEHOLDER (libraryService.ts) — one convention, both credential
// stores, both providers.
const secretPlaceholder = "__WKS_SECRET__"

// redactMcp masks the two MCP fields whose PURPOSE is credentials — `env` (a
// stdio server's API token) and `headers` (an http server's Authorization) —
// which are typed in by hand and written PLAINTEXT into markdown frontmatter,
// including under `<cwd>/.workspacer/library/`, a per-repo directory meant to
// be committed.
//
// Keys stay visible (which variables a server needs is configuration, and the
// UI must render the row to let the user replace it); `url` is deliberately not
// masked — it identifies the server in the list and a credential belongs in
// `headers`. Twin of redactMcp in libraryService.ts; the two must agree or the
// same item comes back masked from one provider and in the clear from the other.
func redactMcp(c *mcpConfig) *mcpConfig {
	if c == nil {
		return nil
	}
	mask := func(in map[string]string) map[string]string {
		if in == nil {
			return nil
		}
		out := make(map[string]string, len(in))
		for k, v := range in {
			if v != "" {
				out[k] = secretPlaceholder
			} else {
				out[k] = v
			}
		}
		return out
	}
	cp := *c
	cp.Env = mask(c.Env)
	cp.Headers = mask(c.Headers)
	return &cp
}

// redactItem is one item as it may leave the process.
func redactItem(it libraryItem) libraryItem {
	if it.Kind != "mcp" || it.Mcp == nil {
		return it
	}
	it.Mcp = redactMcp(it.Mcp)
	return it
}

// restoreSecrets puts the real value back wherever the caller echoed the
// placeholder — the write half of the masked, write-only UI. Without it a
// round-trip through the Library pane (open an MCP item, edit the title, save)
// persists the literal placeholder as the token and breaks the server. A
// placeholder with nothing stored behind it is DROPPED, so a caller cannot
// inject the sentinel as a real value. Twin of restoreSecrets in libraryService.ts.
func restoreSecrets(next *mcpConfig, stored *mcpConfig) *mcpConfig {
	if next == nil {
		return nil
	}
	merge := func(incoming, prev map[string]string) map[string]string {
		if incoming == nil {
			return nil
		}
		out := make(map[string]string, len(incoming))
		for k, v := range incoming {
			if v != secretPlaceholder {
				out[k] = v
				continue
			}
			if kept, ok := prev[k]; ok && kept != secretPlaceholder {
				out[k] = kept
			}
		}
		return out
	}
	cp := *next
	if stored != nil {
		cp.Env = merge(next.Env, stored.Env)
		cp.Headers = merge(next.Headers, stored.Headers)
	} else {
		cp.Env = merge(next.Env, nil)
		cp.Headers = merge(next.Headers, nil)
	}
	return &cp
}

// storedMcpAt returns the MCP config already written at `full`, if any.
func storedMcpAt(full string) *mcpConfig {
	raw, err := os.ReadFile(full)
	if err != nil {
		return nil // new file, or unreadable — nothing to preserve
	}
	data, _ := parseFrontmatter(string(raw))
	return toMcp(data["mcp"])
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
	Title        string         `yaml:"title"`
	Kind         string         `yaml:"kind"`
	Description  string         `yaml:"description,omitempty"`
	Tags         []string       `yaml:"tags,omitempty"`
	Action       string         `yaml:"action,omitempty"`
	Mcp          *mcpConfig     `yaml:"mcp,omitempty"`
	ResultSchema map[string]any `yaml:"resultSchema,omitempty"`
}

func serializeItem(it *libraryItem) string {
	fm := libFrontmatter{Title: it.Title, Kind: it.Kind, Description: it.Description, Tags: it.Tags, Action: it.Action}
	if it.Kind == "mcp" {
		fm.Mcp = cleanMcp(it.Mcp)
	}
	if it.Kind == "dispatch" {
		fm.ResultSchema = it.ResultSchema
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
	if s, ok := k.(string); ok && (s == "skill" || s == "agent" || s == "mcp" || s == "dispatch") {
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

func readLibraryDir(dir, scope string, guard libraryFileGuard) []libraryItem {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var items []libraryItem
	for _, e := range entries {
		name := e.Name()
		// e.IsDir() follows nothing: os.ReadDir returns the LSTAT type, so a
		// symlink is neither a dir nor skipped here. The guard below is what
		// resolves it.
		if e.IsDir() || !strings.HasSuffix(strings.ToLower(name), ".md") {
			continue
		}
		full, ok := guard(filepath.Join(dir, name))
		if !ok {
			continue // a symlink out of the roots, or onto a credential
		}
		raw, err := os.ReadFile(full)
		if err != nil {
			continue
		}
		data, body := parseFrontmatter(string(raw))
		// trimMDSuffix, not TrimSuffix(".md")/TrimSuffix(".MD"): the ENTRY
		// FILTER above is already case-insensitive, so "readme.Md" is listed —
		// and the two spelled-out suffixes left ".Md" on, minting id "readme-md"
		// here where the desktop's /\.md$/i mints "readme". Two ids for one file,
		// and on the desktop side the collision with a plain readme.md silently
		// dropped one item out of the list. Same helper readClaudeItems uses.
		id := slugLibrary(trimMDSuffix(name))
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
			Path:        full, // the validated path, so a round-tripping caller stays inside it
		}
		if kind == "mcp" {
			it.Mcp = toMcp(data["mcp"])
		}
		if kind == "dispatch" {
			it.ResultSchema = toSchemaMap(data["resultSchema"])
		}
		items = append(items, it)
	}
	return items
}

// toSchemaMap accepts only a mapping for a dispatch template's resultSchema —
// yaml.v3 decodes string-keyed mappings as map[string]any, which is exactly the
// JSON-marshalable shape the desktop twin stores. Anything else reads as absent.
func toSchemaMap(v any) map[string]any {
	if m, ok := v.(map[string]any); ok && len(m) > 0 {
		return m
	}
	return nil
}

func readClaudeItem(full, id, kind string, guard libraryFileGuard) *libraryItem {
	full, ok := guard(full)
	if !ok {
		return nil
	}
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
		Origin:      claudeOriginProject,
		Editable:    true,
		Body:        reLeadingBlank.ReplaceAllString(body, ""),
		Path:        full,
	}
}

// readClaudeItems lists the project's Claude Code assets.
//
// The id of a claude item is its REAL ON-DISK BASENAME (skill directory name, or
// agent/command filename sans .md), NOT a slug of it — the same rule
// libraryService.ts states and for the same two reasons. Slugging loses the 1:1
// map back to disk: two names that slug to the same id collide on listLibrary's
// map key and one of them silently disappears from the list, and save/remove
// re-slugging a supplied id then miss the real path. This side slugged and the
// desktop did not, so a project holding both `My.Skill` and `my-skill` listed
// ONE item here and two there, and library.remove(id="My.Skill") re-slugged to
// `my-skill` and os.RemoveAll'd a skill the caller never named while leaving the
// one it did. The trigger is any uppercase letter, dot or space in a skill name.
func readClaudeItems(cwd string, guard libraryFileGuard) []libraryItem {
	var items []libraryItem
	if entries, err := os.ReadDir(claudeSkillsDir(cwd)); err == nil {
		for _, e := range entries {
			if !e.IsDir() {
				continue
			}
			if it := readClaudeItem(filepath.Join(claudeSkillsDir(cwd), e.Name(), "SKILL.md"), e.Name(), "skill", guard); it != nil {
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
			if it := readClaudeItem(filepath.Join(claudeAgentsDir(cwd), name), trimMDSuffix(name), "agent", guard); it != nil {
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
			if it := readClaudeItem(filepath.Join(claudeCommandsDir(cwd), name), trimMDSuffix(name), "command", guard); it != nil {
				items = append(items, *it)
			}
		}
	}
	return items
}

// listLibrary merges global + project (project wins on id) + claude (namespaced),
// sorted by title. Seeds the global dir with any starter it has never seeded.
func listLibrary(cwd string, guard libraryFileGuard) []libraryItem {
	seedLibraryStarters()
	byID := map[string]libraryItem{}
	order := []string{}
	put := func(key string, it libraryItem) {
		if _, ok := byID[key]; !ok {
			order = append(order, key)
		}
		byID[key] = it
	}
	// The GLOBAL dir is guarded too, not just the project ones: <configDir>/
	// library is the one directory a remote caller can write into, so a symlink
	// planted there aimed at the sibling remote-token is the shortest version of
	// the same attack.
	for _, it := range readLibraryDir(libraryGlobalDir(), "global", guard) {
		put(it.ID, it)
	}
	if cwd != "" {
		for _, it := range readLibraryDir(libraryProjectDir(cwd), "project", guard) {
			put(it.ID, it)
		}
		for _, it := range readClaudeItems(cwd, guard) {
			put("claude:"+it.Kind+":"+it.ID, it)
		}
	}
	// Redacted on the way out. Unlike the desktop, this side has no
	// listWithSecrets counterpart and needs none: the brain answers bus calls
	// only, and the two consumers of real MCP credentials (claudeSpawn /
	// managedSpawn) both live in the desktop main process and resolve the
	// configs there from `mcpItemIds`. If a spawn path is ever added here, it
	// must read the files directly rather than relaxing this.
	out := make([]libraryItem, 0, len(byID))
	for _, k := range order {
		out = append(out, redactItem(byID[k]))
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
	// ResultSchema rides only for kind "dispatch"; see libraryItem.ResultSchema.
	ResultSchema map[string]any `json:"resultSchema"`
	Origin       string         `json:"origin"`
	Body         string         `json:"body"`
	Cwd          string         `json:"cwd"`
}

// saveLibrary writes one library item. It is a registry method because the
// project/claude scopes write relative to a CALLER-SUPPLIED cwd, which has to go
// through the same fsguard containment as fs.write — the caller is a bus client,
// and "where do I put this file" is not a question it gets to answer freely.
// (No caller reaching here is unprivileged today: they all hold terminals.create
// or a trusted conn. The point is that the two path-taking surfaces can't drift
// apart, the way the fs.* guard drifted from the desktop's.)
func (r *registry) saveLibrary(ctx context.Context, in libraryInput) (*libraryItem, error) {
	if in.Scope == "claude" {
		return r.saveLibraryClaude(ctx, in)
	}
	// The cwd is confined FIRST, against the workspace roots, and the canonical
	// answer is what the destination is composed from — same two-step as the
	// desktop's guardLibraryCwd + guardLibraryFile. Composing from the caller's
	// raw string and checking only the result would let filepath.Join Clean a
	// `link/..` escape away before the guard ever saw it.
	cwd := firstNonEmpty(in.Cwd, mustCwd())
	canonicalCwd := ""
	if in.Scope == "project" {
		var err error
		if canonicalCwd, err = assertPathAllowed("library.save", cwd, r.workspaceRoots(ctx)); err != nil {
			return nil, err
		}
	}
	dir := libraryGlobalDir()
	if in.Scope == "project" {
		dir = libraryProjectDir(canonicalCwd)
	}
	id := slugLibrary(firstNonEmpty(in.ID, in.Title))
	full := filepath.Join(dir, id+".md")
	// Checked BEFORE MkdirAll, so a denied save leaves no directories behind.
	// The ITEM roots, not the workspace roots: where a library item may
	// legitimately live is the global store plus the project the caller named,
	// and nothing else — see libraryItemRoots.
	canonical, err := assertLibraryItemPath("library.save", full, canonicalCwd)
	if err != nil {
		return nil, err
	}
	// Create and write exactly what was validated. A symlinked component makes
	// the checked path and the opened path two different files otherwise, and
	// `dir` has to be re-derived or MkdirAll would rebuild the unresolved one.
	full = canonical
	dir = filepath.Dir(full)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return nil, err
	}
	it := &libraryItem{
		ID: id, Scope: in.Scope, Title: in.Title, Kind: in.Kind,
		Description: in.Description, Tags: in.Tags, Action: in.Action,
		Body: in.Body, Path: full,
	}
	if in.Kind == "dispatch" {
		it.ResultSchema = in.ResultSchema
	}
	if in.Kind == "mcp" {
		// Echoed placeholders resolve against what is already on disk, so a save
		// that only touched the title keeps the token.
		it.Mcp = cleanMcp(restoreSecrets(in.Mcp, storedMcpAt(full)))
	}
	if err := writeFileAtomic(full, []byte(serializeItem(it)), 0o644); err != nil {
		return nil, err
	}
	// Masked on the way back out, like listLibrary: this return value goes
	// straight to the bus caller.
	redacted := redactItem(*it)
	return &redacted, nil
}

func (r *registry) saveLibraryClaude(ctx context.Context, in libraryInput) (*libraryItem, error) {
	// Before the cwd is even resolved: a plugin's assets are never a write
	// target, and saying so beats composing a path the item guard then refuses
	// for an unrelated-sounding reason.
	if err := assertWritableOrigin(in.Origin); err != nil {
		return nil, err
	}
	canonicalCwd, err := assertPathAllowed("library.save", firstNonEmpty(in.Cwd, mustCwd()), r.workspaceRoots(ctx))
	if err != nil {
		return nil, err
	}
	cwd := canonicalCwd
	kind := "skill"
	if in.Kind == "agent" {
		kind = "agent"
	} else if in.Kind == "command" {
		kind = "command"
	}
	// A claude id is the item's REAL on-disk basename, never a slug of it. This
	// side used to slugLibrary() it while libraryService.ts took it verbatim, so
	// the same params produced two different files: save(id="My.Skill") wrote
	// .claude/skills/my-skill/SKILL.md here and .claude/skills/My.Skill/SKILL.md
	// there. An EXISTING item's id is its basename, so edit it in place; only a
	// brand-new item minted from a title gets slugged. A supplied id is still
	// caller data, so it has to look like a basename — see assertPlainBasename.
	id := slugLibrary(in.Title)
	if in.ID != "" {
		var ok bool
		if id, ok = assertPlainBasename(in.ID); !ok {
			return nil, fmt.Errorf("invalid library item id: %q", in.ID)
		}
	}
	var full string
	switch kind {
	case "skill":
		full = filepath.Join(claudeSkillsDir(cwd), id, "SKILL.md")
	case "command":
		full = filepath.Join(claudeCommandsDir(cwd), id+".md")
	default:
		full = filepath.Join(claudeAgentsDir(cwd), id+".md")
	}
	canonical, err := assertLibraryItemPath("library.save", full, canonicalCwd)
	if err != nil {
		return nil, err
	}
	// Read, create and write the validated path — the returned item's Path is
	// this string too, so a caller round-tripping it stays inside the guard.
	full = canonical
	if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
		return nil, err
	}
	existing := map[string]any{}
	if raw, err := os.ReadFile(full); err == nil {
		existing, _ = parseFrontmatter(string(raw))
	}
	if err := writeFileAtomic(full, []byte(serializeClaude(existing, in.Title, in.Description, in.Body)), 0o644); err != nil {
		return nil, err
	}
	return &libraryItem{ID: id, Scope: "claude", Title: in.Title, Kind: kind, Description: in.Description, Origin: claudeOriginProject, Editable: true, Body: in.Body, Path: full}, nil
}

// removeLibrary deletes one item. The DELETE TARGET is guarded, not the cwd it
// was composed from: `<cwd>/.claude/skills` is a caller-writable location inside
// the root, so pointing it at the config dir with a directory symlink turned a
// library.remove of id "remote-token" into an os.RemoveAll of the bus credential
// — the cwd the guard saw was impeccable. os.RemoveAll in particular does not
// follow the final symlink but DOES traverse symlinked parents, so the whole
// derived path has to be canonical before anything is unlinked.
func removeLibrary(scope, id, cwd, kind, origin string, guard libraryFileGuard) error {
	// Refused before anything is derived — the skill branch below is a
	// recursive RemoveAll, and a plugin's skill directory is part of an
	// installed package, not the caller's library. Loud, not silent: a caller
	// asking to delete a plugin's skill needs to hear no, and the item guard
	// would otherwise just drop it and report success.
	if scope == "claude" {
		if err := assertWritableOrigin(origin); err != nil {
			return err
		}
	}
	remove := func(path string, recursive bool) {
		canonical, ok := guard(path)
		if !ok {
			return
		}
		if recursive {
			_ = os.RemoveAll(canonical)
			return
		}
		_ = os.Remove(canonical)
	}
	if scope == "claude" {
		root := firstNonEmpty(cwd, mustCwd())
		// The id is the item's real on-disk basename (readClaudeItems), so it is
		// used verbatim rather than re-slugged: slugging deleted a DIFFERENT
		// skill than the one named — remove(id="My.Skill") unlinked `my-skill`
		// and left `My.Skill` standing — while a name that is not slug-stable
		// unlinked nothing at all. Verbatim means caller data reaches
		// filepath.Join, so it must look like a basename first.
		name, ok := assertPlainBasename(id)
		if !ok {
			return nil
		}
		switch kind {
		case "agent":
			remove(filepath.Join(claudeAgentsDir(root), name+".md"), false)
		case "command":
			remove(filepath.Join(claudeCommandsDir(root), name+".md"), false)
		default:
			remove(filepath.Join(claudeSkillsDir(root), name), true)
		}
		return nil
	}
	dir := libraryGlobalDir()
	if scope == "project" {
		dir = libraryProjectDir(firstNonEmpty(cwd, mustCwd()))
	}
	remove(filepath.Join(dir, slugLibrary(id)+".md"), false)
	return nil
}

// preMarkerStarterIDs are the starters that shipped BEFORE library-seeded.json
// existed, and the only reason that file needs a bootstrap rule at all.
//
// An install predating the marker has demonstrably been offered these four (the
// old all-or-nothing seeder wrote them on its first run or not at all), so one
// of them missing from a NON-EMPTY library means the user deleted it, and the
// seeder must not put it back. A starter not in this list postdates the marker,
// has never been offered to such an install, and its absence means nothing.
//
// Frozen by definition: never add to it. A new starter belongs in starterItems
// only, which is exactly what makes it seed for existing users. The TS twin's
// PRE_MARKER_STARTER_IDS is the same list.
var preMarkerStarterIDs = []string{
	"summarize-and-plan",
	"careful-refactor",
	"context7-mcp",
	"make-workspacer-plugin",
}

// starterItem is one starter and the file id (<id>.md) it is written as.
type starterItem struct {
	ID   string
	Item libraryItem
}

// librarySeedStatePath is where "we have already offered this starter" is
// recorded: a small JSON file beside the library dir, in the same shape as the
// config store's other sidecars (peers.json, claude-profiles.json).
//
// It exists because the only other available signal — is the file on disk? —
// cannot tell "you have never been offered this" apart from "I deleted it on
// purpose", and the seeder must never undo the second one. The TS twin
// (libraryService.ts seedStatePath) reads and writes this same file with the
// same key, so whichever process runs first records for both.
func librarySeedStatePath() string { return filepath.Join(configDir(), "library-seeded.json") }

// readLibrarySeedState returns the ids ever seeded, or nil when the marker has
// never been written. Unreadable or malformed reads as nil: re-offering the
// post-marker starters is recoverable, and the bootstrap in seedLibraryStarters
// still protects the pre-marker four from being resurrected.
func readLibrarySeedState() map[string]bool {
	raw, err := os.ReadFile(librarySeedStatePath())
	if err != nil {
		return nil
	}
	var state struct {
		Seeded []string `json:"seeded"`
	}
	if json.Unmarshal(raw, &state) != nil || state.Seeded == nil {
		return nil
	}
	seeded := map[string]bool{}
	for _, id := range state.Seeded {
		seeded[id] = true
	}
	return seeded
}

// seedLibraryStarters seeds every starter that has never been seeded and is not
// already on disk, the same set (and order) the app ships (seedGlobalStarters).
//
// This was seedLibraryIfEmpty, which returned the moment the global dir held
// ANY .md — so a starter added after a user's first run (the three dispatch
// templates, most recently) stayed invisible forever to every existing install.
// Seeding is per-ITEM now. Two rules it must not break: never overwrite a file
// that exists (the user may have edited it), and never resurrect one the user
// DELETED — which is what the marker buys. A genuinely empty dir still gets the
// whole set, exactly as before. Best-effort and idempotent.
func seedLibraryStarters() {
	dir := libraryGlobalDir()
	recorded := readLibrarySeedState()
	seeded := recorded
	if seeded == nil {
		// No marker yet: a non-empty library is a pre-marker install, so treat
		// the starters that shipped before the marker as already offered. An
		// empty one is a true first run.
		seeded = map[string]bool{}
		if entries, err := os.ReadDir(dir); err == nil {
			for _, e := range entries {
				if strings.HasSuffix(strings.ToLower(e.Name()), ".md") {
					for _, id := range preMarkerStarterIDs {
						seeded[id] = true
					}
					break
				}
			}
		}
	}
	var fresh []starterItem
	for _, s := range starterItems() {
		if !seeded[s.ID] {
			fresh = append(fresh, s)
		}
	}
	// Idempotent fast path: every run after the first has nothing to seed and
	// nothing to record, and touches no files at all. listLibrary calls this on
	// EVERY call, so the steady state must stay a single stat.
	if len(fresh) == 0 && recorded != nil {
		return
	}
	if os.MkdirAll(dir, 0o755) != nil {
		return
	}
	for _, s := range fresh {
		full := filepath.Join(dir, s.ID+".md")
		// An existing file is the user's, even when the marker has never seen
		// it. It is still recorded below — just never written over.
		if _, err := os.Stat(full); err == nil {
			continue
		}
		item := s.Item
		_ = os.WriteFile(full, []byte(serializeItem(&item)), 0o644)
	}
	// Record everything offered on this pass, written or skipped, so that
	// deleting it afterwards keeps it gone.
	for _, s := range fresh {
		seeded[s.ID] = true
	}
	ids := make([]string, 0, len(seeded))
	for id := range seeded {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	if blob, err := json.MarshalIndent(struct {
		Seeded []string `json:"seeded"`
	}{ids}, "", "  "); err == nil {
		_ = writeFileAtomic(librarySeedStatePath(), append(blob, '\n'), 0o644)
	}
}

// starterItems is the starter library, by file id. The TS twin ships the same
// set in the same order (libraryService.ts starters()); the seed-count tests on
// both sides pin that they agree.
func starterItems() []starterItem {
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
		// Dispatch templates (kind "dispatch") — the Fleet Manager's reusable
		// dispatch framing, rendered host-side by the DESKTOP's agents.spawn
		// {template, templateParams} (lib/dispatchTemplate.ts; the brain
		// declines those spawn params — see parity_test.go). Seeded here too so
		// the twins ship the same first-run library. {{task}} is REQUIRED by
		// design: the manager writes the task-specific reasoning; only the
		// framing is canned.
		{
			Title: "Ship task (dispatch)", Kind: "dispatch",
			Description: "Delivery-mode boilerplate + reporting contract for a worker that changes code. Fill {{task}}; {{delivery}} defaults to opening a PR.",
			Tags:        []string{"dispatch", "ship"},
			ResultSchema: map[string]any{
				"type":     "object",
				"required": []any{"commit"},
				"properties": map[string]any{
					"commit":       map[string]any{"type": "string"},
					"filesChanged": map[string]any{"type": "array", "items": map[string]any{"type": "string"}},
					"checksRun":    map[string]any{"type": "array", "items": map[string]any{"type": "string"}},
					"caveats":      map[string]any{"type": "string"},
				},
			},
			Body: strings.Join([]string{
				"SHIP TASK in {{cwd}}.",
				"",
				"{{task}}",
				"",
				"Ground rules:",
				"- Work only inside this repo. Never push unless the task above says to.",
				"- Deliver the result this way: {{delivery:open a pull request for the user to review; do not merge it yourself}}.",
				"- Run the project’s own checks (build, tests, lint) on what you changed before reporting, and use the repo’s code-intelligence tools (CLAUDE.md / AGENTS.md names them) instead of blind grep.",
				"",
				"When you are done, end your turn with a short report: what you did, the commit id, the files you changed, which checks you ran, and any caveats. That final message reaches your manager automatically; do not try to message anyone, just finish.",
			}, "\n"),
		},
		{
			Title: "Scout task (dispatch)", Kind: "dispatch",
			Description: "Read-only investigation framing + report-to-file contract. Fill {{task}}; {{reportPath}} defaults to a dated file under .workspacer/reports/.",
			Tags:        []string{"dispatch", "scout"},
			ResultSchema: map[string]any{
				"type":     "object",
				"required": []any{"findings"},
				"properties": map[string]any{
					"findings":   map[string]any{"type": "string"},
					"reportPath": map[string]any{"type": "string"},
					"followUps":  map[string]any{"type": "array", "items": map[string]any{"type": "string"}},
				},
			},
			Body: strings.Join([]string{
				"SCOUT TASK in {{cwd}} — investigate only. Do not edit source, run builds that write artifacts into the repo, or push anything.",
				"",
				"{{task}}",
				"",
				"Write your full findings to {{reportPath:.workspacer/reports/<YYYY-MM-DD>-<topic>.md}} so they outlive this session, then end your turn with a short summary: the answer, the report path, and any follow-ups you would dispatch. Your final message reaches your manager automatically; just finish.",
			}, "\n"),
		},
		{
			Title: "Two explanations (dispatch)", Kind: "dispatch",
			Description: "Diagnose-before-fixing scaffold: name two opposite explanations for a symptom and make the worker establish which holds before changing anything.",
			Tags:        []string{"dispatch", "diagnose"},
			ResultSchema: map[string]any{
				"type":     "object",
				"required": []any{"verdict", "evidence"},
				"properties": map[string]any{
					"verdict":  map[string]any{"type": "string"},
					"evidence": map[string]any{"type": "string"},
					"fix":      map[string]any{"type": "string"},
					"caveats":  map[string]any{"type": "string"},
				},
			},
			Body: strings.Join([]string{
				"DIAGNOSE BEFORE FIXING, in {{cwd}}.",
				"",
				"The symptom: {{symptom}}",
				"",
				"There are two opposite explanations, with opposite fixes:",
				"(A) {{explanationA}}",
				"(B) {{explanationB}}",
				"",
				"Establish WHICH ONE holds before you change anything, and say what evidence settled it. If the evidence shows neither holds, that is a SUCCESS, not a failure: report what you found and stop rather than forcing a fix. Only then apply the fix that matches the verdict: {{fixInstruction:apply the smallest fix that matches the verdict, run the relevant checks, and report}}.",
				"",
				"End your turn with the verdict, the evidence, and what you did about it.",
			}, "\n"),
		},
	}
	// Positional pairing, as before — ids[i] names seeds[i]. A mismatched length
	// is a programming error caught by the seed-count test, not a runtime case.
	ids := []string{"summarize-and-plan", "careful-refactor", "context7-mcp", "make-workspacer-plugin", "ship-task", "scout-task", "two-explanations"}
	out := make([]starterItem, 0, len(seeds))
	for i := range seeds {
		if i >= len(ids) {
			break
		}
		out = append(out, starterItem{ID: ids[i], Item: seeds[i]})
	}
	return out
}

func mustCwd() string {
	cwd, err := os.Getwd()
	if err != nil {
		return ""
	}
	return cwd
}

// firstNonEmpty returns the first value that holds something other than
// whitespace. The TRIM is load-bearing and not cosmetic: this backs the library
// title/name fallbacks, whose desktop twin is
// `typeof t === 'string' && hasNonBlankText(t) ? t : id` (libraryService.ts,
// where hasNonBlankText trims the same asciiWhitespace set). Without it a frontmatter
// `title: "   "` served three spaces here and "wsp" there — a blank row in the
// library picker under the default catalog delegation and a named row without
// it. It is also the cwd fallback (`firstNonEmpty(in.Cwd, mustCwd())`), where a
// whitespace-only cwd is refused by the guard either way.
func firstNonEmpty(vals ...string) string {
	for _, v := range vals {
		// ASCII-blank, not TrimSpace-blank: a pure-BOM (U+FEFF) title is kept by
		// TrimSpace but dropped by the desktop twin's `.trim()`, and a pure-NEL
		// (U+0085) title the other way. Both copies use the shared asciiWhitespace
		// set so the SAME title survives the fallback on both providers (and thus
		// sorts to the same byteCompare slot in the library picker).
		if strings.Trim(v, asciiWhitespace) != "" {
			return v
		}
	}
	return ""
}
