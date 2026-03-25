package main

import (
	"context"
	"fmt"
	"net/url"
	"strings"
	"sync"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// browserWindow tracks a browser window and its current URL.
type browserWindow struct {
	url    string
	window *application.WebviewWindow
}

// BrowserService manages browser windows (separate Wails webview windows
// that navigate to real URLs).
type BrowserService struct {
	mu      sync.Mutex
	windows map[string]*browserWindow
}

// ServiceStartup initializes the BrowserService when the application starts.
func (bs *BrowserService) ServiceStartup(ctx context.Context, options application.ServiceOptions) error {
	bs.windows = make(map[string]*browserWindow)
	return nil
}

// ServiceShutdown cleans up all browser windows when the application shuts down.
func (bs *BrowserService) ServiceShutdown() error {
	bs.mu.Lock()
	defer bs.mu.Unlock()

	for id, bw := range bs.windows {
		bw.window.Close()
		delete(bs.windows, id)
	}
	return nil
}

// normalizeURL ensures the URL has an http:// or https:// scheme.
func normalizeURL(rawURL string) string {
	rawURL = strings.TrimSpace(rawURL)
	if !strings.HasPrefix(rawURL, "http://") && !strings.HasPrefix(rawURL, "https://") {
		rawURL = "https://" + rawURL
	}
	return rawURL
}

// windowTitle extracts a short title from a URL (the host portion),
// falling back to the full URL if parsing fails.
func windowTitle(rawURL string) string {
	parsed, err := url.Parse(rawURL)
	if err != nil || parsed.Host == "" {
		return rawURL
	}
	return parsed.Host
}

// OpenBrowser opens a new Wails window navigated to the given URL.
// If a window already exists for the given paneId, it navigates
// the existing window to the new URL instead of creating a new one.
func (bs *BrowserService) OpenBrowser(paneId string, urlStr string) error {
	if urlStr == "" {
		return fmt.Errorf("url must not be empty")
	}

	urlStr = normalizeURL(urlStr)
	title := windowTitle(urlStr)

	bs.mu.Lock()
	defer bs.mu.Unlock()

	if bw, exists := bs.windows[paneId]; exists {
		// Navigate existing window to the new URL.
		bw.window.SetURL(urlStr)
		bw.url = urlStr
		return nil
	}

	// Create a new browser window.
	app := application.Get()
	if app == nil {
		return fmt.Errorf("application not available")
	}

	win := app.Window.NewWithOptions(application.WebviewWindowOptions{
		Title:     title,
		Width:     1200,
		Height:    800,
		MinWidth:  400,
		MinHeight: 300,
		URL:       urlStr,
		Frameless: false,
	})

	bs.windows[paneId] = &browserWindow{
		url:    urlStr,
		window: win,
	}

	return nil
}

// CloseBrowser closes the browser window for the given pane and removes it.
func (bs *BrowserService) CloseBrowser(paneId string) error {
	bs.mu.Lock()
	bw, exists := bs.windows[paneId]
	if !exists {
		bs.mu.Unlock()
		return fmt.Errorf("no browser window for pane: %s", paneId)
	}
	delete(bs.windows, paneId)
	bs.mu.Unlock()

	bw.window.Close()
	return nil
}

// GetBrowserURL returns the current URL for the pane's browser window.
// Returns an empty string if no browser window exists for the pane.
func (bs *BrowserService) GetBrowserURL(paneId string) string {
	bs.mu.Lock()
	defer bs.mu.Unlock()

	if bw, exists := bs.windows[paneId]; exists {
		return bw.url
	}
	return ""
}
