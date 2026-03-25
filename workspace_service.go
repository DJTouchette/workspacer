package main

// PaneConfig describes the configuration for a single pane in the workspace layout.
type PaneConfig struct {
	// ID is a unique identifier for the pane.
	ID string `json:"id"`

	// Type is the kind of pane: "terminal", "browser", "notes", or "agent".
	Type string `json:"type"`

	// Title is the display title for the pane tab/header.
	Title string `json:"title"`

	// Width is the default width of the pane in pixels.
	Width int `json:"width"`

	// Order determines the left-to-right position of the pane.
	Order int `json:"order"`
}

// WorkspaceService provides methods for managing the workspace pane layout.
type WorkspaceService struct{}

// GetPaneLayout returns the current pane layout configuration.
// For now this returns the default set of panes.
func (ws *WorkspaceService) GetPaneLayout() []PaneConfig {
	return ws.GetDefaultPanes()
}

// GetDefaultPanes returns the default workspace layout:
// three terminal panes and one notes pane.
func (ws *WorkspaceService) GetDefaultPanes() []PaneConfig {
	return []PaneConfig{
		{
			ID:    "term-1",
			Type:  "terminal",
			Title: "Terminal 1",
			Width: 600,
			Order: 0,
		},
		{
			ID:    "term-2",
			Type:  "terminal",
			Title: "Terminal 2",
			Width: 600,
			Order: 1,
		},
		{
			ID:    "term-3",
			Type:  "terminal",
			Title: "Terminal 3",
			Width: 600,
			Order: 2,
		},
		{
			ID:    "notes-1",
			Type:  "notes",
			Title: "Notes",
			Width: 400,
			Order: 3,
		},
	}
}
