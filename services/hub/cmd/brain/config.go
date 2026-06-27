package main

// config.* — the workspacer config document (theme, keybindings, pane settings,
// claude defaults, …). A faithful Go port of configService.ts: the same
// config.yaml at ~/.config/workspacer, the same deep-merge-over-defaults on
// read, the same one-time keybindings migration, so a headless client (web,
// TUI) loads the *real* config instead of falling back to renderer defaults.
//
// Config is kept generic (map[string]any) rather than a typed struct: that
// mirrors the TS deepMerge's object semantics exactly and means a new config key
// added on the app side flows through without a matching Go change. The defaults
// are embedded as JSON below — a 1:1 transcription of configService.defaultConfig().

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"

	yaml "gopkg.in/yaml.v3"
)

// defaultConfigJSON mirrors configService.defaultConfig() (the non-Windows
// shell list; Windows host support can come later). Keep in sync with
// apps/desktop/src/main/services/configService.ts.
const defaultConfigJSON = `{
  "ui": {
    "animations": false, "theme": "dark", "cornerStyle": "", "borderColor": "",
    "fontFamily": "Inter, system-ui, sans-serif", "fontSize": 14, "borderRadius": 8,
    "navBarHeight": 34, "paneHeaderHeight": 22, "showComposerSend": true, "guiFontScale": 1.15
  },
  "terminal": {
    "shell": "",
    "shells": [
      { "name": "default", "path": "", "label": "Default ($SHELL)" },
      { "name": "bash", "path": "/bin/bash", "label": "Bash" },
      { "name": "zsh", "path": "/bin/zsh", "label": "Zsh" },
      { "name": "fish", "path": "/usr/bin/fish", "label": "Fish" }
    ],
    "fontFamily": "\"JetBrainsMono Nerd Font Mono\", \"JetBrainsMonoNL Nerd Font Mono\", \"JetBrainsMono NFM\", \"JetBrainsMonoNL NFM\", \"JetBrainsMono NF\", \"CaskaydiaMono Nerd Font Mono\", \"CaskaydiaCove Nerd Font Mono\", \"CaskaydiaMono NF\", \"Cascadia Mono\", monospace",
    "fontSize": 14, "scrollback": 1500, "cursorBlink": true, "cursorStyle": "block"
  },
  "browser": {
    "homepage": "https://google.com",
    "bookmarks": [
      { "name": "Go Docs", "url": "https://pkg.go.dev" },
      { "name": "MDN", "url": "https://developer.mozilla.org" },
      { "name": "Localhost 3000", "url": "http://localhost:3000" },
      { "name": "Localhost 8080", "url": "http://localhost:8080" }
    ],
    "hibernateAfter": 300
  },
  "panes": {
    "defaultWidth": 800, "gap": 16, "peek": 80, "insertPosition": "after",
    "tabPosition": "top", "viewMode": "tabs", "viewLevel": "piloting",
    "default": [
      { "id": "terminal-1", "type": "terminal", "title": "Terminal 1", "width": 800, "order": 0 },
      { "id": "terminal-2", "type": "terminal", "title": "Terminal 2", "width": 800, "order": 1 },
      { "id": "terminal-3", "type": "terminal", "title": "Terminal 3", "width": 800, "order": 2 },
      { "id": "notes-1", "type": "notes", "title": "Notes", "width": 800, "order": 3 }
    ]
  },
  "keybindings": {
    "prefix": "ctrl+space", "chordHints": true,
    "shortcuts": {
      "command-palette": "ctrl+shift+p", "next-agent": "ctrl+tab", "prev-agent": "ctrl+shift+tab",
      "next-attention": "ctrl+shift+space", "spawn-agent": "ctrl+shift+n", "settings": "ctrl+,",
      "save-session": "ctrl+shift+s", "open-file": "ctrl+shift+o", "toggle-help": "f1",
      "toggle-terminal": "ctrl+` + "`" + `", "toggle-sidebar": "ctrl+shift+b", "toggle-inbox": "ctrl+shift+i",
      "toggle-fleet": "ctrl+shift+f", "toggle-inspector": "ctrl+shift+e", "library-picker": "ctrl+shift+l",
      "open-review": "ctrl+shift+g", "new-terminal": "prefix n t", "new-claude": "prefix n c",
      "new-browser": "prefix n b", "prev-tab": "prefix t [", "next-tab": "prefix t ]",
      "move-tab-left": "prefix t ,", "move-tab-right": "prefix t .", "rename-tab": "prefix t r",
      "close-pane": "prefix t w", "split": "prefix p s", "quick-split": "prefix p c",
      "nav-left": "prefix p h", "nav-down": "prefix p j", "nav-up": "prefix p k",
      "nav-right": "prefix p l", "cycle-view": "prefix v"
    }
  },
  "notifications": { "enabled": true, "notifyDone": true, "onlyWhenUnwatched": true, "sound": false },
  "editor": { "engine": "codemirror", "terminalCommand": "nvim" },
  "claude": { "defaultModel": "", "seenModels": [], "skipPermissionsDefault": false, "defaultView": "terminal" },
  "supervisor": { "model": "", "summarizerModel": "sonnet", "pollSeconds": 45 },
  "directories": { "recent": [], "favourites": [] },
  "scripts": {},
  "session": { "autoResume": false },
  "apps": [
    { "name": "GitHub", "url": "https://github.com", "icon": "💻" },
    { "name": "ChatGPT", "url": "https://chat.openai.com", "icon": "🤖" },
    { "name": "Claude", "url": "https://claude.ai", "icon": "✨" },
    { "name": "Stack Overflow", "url": "https://stackoverflow.com", "icon": "📚" },
    { "name": "Localhost 3000", "url": "http://localhost:3000", "icon": "🌐" },
    { "name": "Localhost 8080", "url": "http://localhost:8080", "icon": "🌐" }
  ]
}`

func defaultConfig() map[string]any {
	var m map[string]any
	_ = json.Unmarshal([]byte(defaultConfigJSON), &m)
	return m
}

func configPath() string {
	return filepath.Join(configDir(), "config.yaml")
}

// deepMerge overlays source onto a shallow copy of target. A null source value
// means "unset" (skip, keep the default); nested maps recurse; everything else
// (incl. arrays) replaces. Mirrors configService.deepMerge.
func deepMerge(target, source map[string]any) map[string]any {
	result := make(map[string]any, len(target))
	for k, v := range target {
		result[k] = v
	}
	for k, sv := range source {
		if sv == nil {
			continue
		}
		if svMap, ok := sv.(map[string]any); ok {
			if tvMap, ok := result[k].(map[string]any); ok {
				result[k] = deepMerge(tvMap, svMap)
				continue
			}
		}
		result[k] = sv
	}
	return result
}

// configService caches the merged config and serializes access, since bus calls
// are handled on concurrent goroutines. Mirrors the TS singleton.
type configService struct {
	mu      sync.Mutex
	current map[string]any
}

func newConfigService() *configService {
	c := &configService{}
	c.current = c.loadFromDisk()
	return c
}

func (c *configService) loadFromDisk() map[string]any {
	defaults := defaultConfig()
	data, err := os.ReadFile(configPath())
	if err != nil {
		c.writeDefaults(defaults)
		return defaults
	}
	var parsed map[string]any
	if err := yaml.Unmarshal(data, &parsed); err != nil {
		return defaults
	}
	return migrateKeybindings(deepMerge(defaults, parsed))
}

func (c *configService) writeDefaults(defaults map[string]any) {
	writeConfigYAML(defaults)
}

func (c *configService) get() map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.current
}

func (c *configService) reload() map[string]any {
	cfg := c.loadFromDisk()
	c.mu.Lock()
	c.current = cfg
	c.mu.Unlock()
	return cfg
}

func (c *configService) save(partial map[string]any) map[string]any {
	c.mu.Lock()
	merged := deepMerge(c.current, partial)
	c.current = merged
	c.mu.Unlock()
	writeConfigYAML(merged)
	return merged
}

func (c *configService) path() string { return configPath() }

func writeConfigYAML(cfg map[string]any) {
	if err := os.MkdirAll(configDir(), 0o755); err != nil {
		return
	}
	data, err := yaml.Marshal(cfg)
	if err != nil {
		return
	}
	_ = os.WriteFile(configPath(), data, 0o644)
}

// migrateKeybindings ports configService.migrateKeybindings: the old
// mode/leader scheme (or a missing prefix) is reset to the prefix-forward
// defaults, preserving Vim mode as editor.vim. Idempotent — runs once because
// the rewrite leaves a valid prefix and no mode/leader.
func migrateKeybindings(cfg map[string]any) map[string]any {
	kb, _ := cfg["keybindings"].(map[string]any)
	if kb == nil {
		return cfg
	}
	_, hasMode := kb["mode"]
	_, hasLeader := kb["leader"]
	prefix, _ := kb["prefix"].(string)
	legacy := hasMode || hasLeader || prefix == ""
	if !legacy {
		return cfg
	}
	hadVim := false
	if m, ok := kb["mode"].(string); ok && m == "vim" {
		hadVim = true
	}
	def := defaultConfig()
	cfg["keybindings"] = def["keybindings"]
	if hadVim {
		ed, _ := cfg["editor"].(map[string]any)
		if ed == nil {
			ed = map[string]any{}
		}
		ed["vim"] = true
		cfg["editor"] = ed
	}
	writeConfigYAML(cfg)
	return cfg
}
