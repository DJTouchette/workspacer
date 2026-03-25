package main

import (
	"os"
	"path/filepath"
	"runtime"

	"gopkg.in/yaml.v3"
)

// Config holds all user-configurable settings for Workspacer.
type Config struct {
	UI       UIConfig       `yaml:"ui" json:"ui"`
	Terminal TerminalConfig `yaml:"terminal" json:"terminal"`
	Panes    PanesConfig    `yaml:"panes" json:"panes"`
}

type UIConfig struct {
	Animations  bool   `yaml:"animations" json:"animations"`
	Theme       string `yaml:"theme" json:"theme"`
	FontFamily  string `yaml:"font_family" json:"fontFamily"`
	FontSize    int    `yaml:"font_size" json:"fontSize"`
	BorderRadius int   `yaml:"border_radius" json:"borderRadius"`
	NavBarHeight    int `yaml:"navbar_height" json:"navBarHeight"`
	PaneHeaderHeight int `yaml:"pane_header_height" json:"paneHeaderHeight"`
}

type TerminalConfig struct {
	Shell       string `yaml:"shell" json:"shell"`
	FontFamily  string `yaml:"font_family" json:"fontFamily"`
	FontSize    int    `yaml:"font_size" json:"fontSize"`
	Scrollback  int    `yaml:"scrollback" json:"scrollback"`
	CursorBlink bool   `yaml:"cursor_blink" json:"cursorBlink"`
	CursorStyle string `yaml:"cursor_style" json:"cursorStyle"`
}

type PanesConfig struct {
	DefaultWidth   int          `yaml:"default_width" json:"defaultWidth"`
	Gap            int          `yaml:"gap" json:"gap"`
	Peek           int          `yaml:"peek" json:"peek"`
	InsertPosition string       `yaml:"insert_position" json:"insertPosition"`
	Default        []PaneConfig `yaml:"default" json:"default"`
}

// DefaultConfig returns the default configuration.
func DefaultConfig() Config {
	return Config{
		UI: UIConfig{
			Animations:   false,
			Theme:        "dark",
			FontFamily:   "Inter, system-ui, sans-serif",
			FontSize:     14,
			BorderRadius: 8,
			NavBarHeight:     28,
			PaneHeaderHeight: 22,
		},
		Terminal: TerminalConfig{
			Shell:       "",
			FontFamily:  "JetBrainsMono NF, JetBrainsMono Nerd Font, CaskaydiaMono NF, monospace",
			FontSize:    14,
			Scrollback:  5000,
			CursorBlink: true,
			CursorStyle: "block",
		},
		Panes: PanesConfig{
			DefaultWidth: 800,
			Gap:          16,
			Peek:           80,
			InsertPosition: "after",
			Default: []PaneConfig{
				{ID: "terminal-1", Type: "terminal", Title: "Terminal 1", Width: 800, Order: 0},
				{ID: "terminal-2", Type: "terminal", Title: "Terminal 2", Width: 800, Order: 1},
				{ID: "terminal-3", Type: "terminal", Title: "Terminal 3", Width: 800, Order: 2},
				{ID: "notes-1", Type: "notes", Title: "Notes", Width: 800, Order: 3},
			},
		},
	}
}

// configDir returns the platform-appropriate config directory.
func configDir() string {
	switch runtime.GOOS {
	case "windows":
		appData := os.Getenv("APPDATA")
		if appData != "" {
			return filepath.Join(appData, "workspacer")
		}
		home, _ := os.UserHomeDir()
		return filepath.Join(home, "AppData", "Roaming", "workspacer")
	default:
		// Linux / macOS: use XDG_CONFIG_HOME or ~/.config
		xdg := os.Getenv("XDG_CONFIG_HOME")
		if xdg != "" {
			return filepath.Join(xdg, "workspacer")
		}
		home, _ := os.UserHomeDir()
		return filepath.Join(home, ".config", "workspacer")
	}
}

// ConfigPath returns the full path to the config file.
func ConfigPath() string {
	return filepath.Join(configDir(), "config.yaml")
}

// LoadConfig reads the config file, falling back to defaults for missing values.
func LoadConfig() Config {
	cfg := DefaultConfig()

	data, err := os.ReadFile(ConfigPath())
	if err != nil {
		// No config file — write the default one
		_ = WriteDefaultConfig()
		return cfg
	}

	_ = yaml.Unmarshal(data, &cfg)
	return cfg
}

// WriteDefaultConfig writes the default config to disk.
func WriteDefaultConfig() error {
	dir := configDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}

	cfg := DefaultConfig()
	data, err := yaml.Marshal(&cfg)
	if err != nil {
		return err
	}

	return os.WriteFile(ConfigPath(), data, 0o644)
}
