package main

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// BrowserService fetches web pages and returns cleaned HTML for display.
type BrowserService struct {
	mu     sync.Mutex
	client *http.Client
	cache  map[string]cachedPage
}

type cachedPage struct {
	html      string
	fetchedAt time.Time
}

func (bs *BrowserService) ServiceStartup(ctx context.Context, options application.ServiceOptions) error {
	bs.client = &http.Client{
		Timeout: 15 * time.Second,
	}
	bs.cache = make(map[string]cachedPage)
	return nil
}

// FetchPage fetches a URL and returns cleaned HTML suitable for rendering.
// Scripts are removed, relative URLs are made absolute, and basic styling is preserved.
func (bs *BrowserService) FetchPage(rawURL string) (string, error) {
	// Normalize URL
	if !strings.HasPrefix(rawURL, "http://") && !strings.HasPrefix(rawURL, "https://") {
		rawURL = "https://" + rawURL
	}

	parsed, err := url.Parse(rawURL)
	if err != nil {
		return "", fmt.Errorf("invalid URL: %w", err)
	}

	// Check cache (30 second TTL)
	bs.mu.Lock()
	if cached, ok := bs.cache[rawURL]; ok && time.Since(cached.fetchedAt) < 30*time.Second {
		bs.mu.Unlock()
		return cached.html, nil
	}
	bs.mu.Unlock()

	req, err := http.NewRequest("GET", rawURL, nil)
	if err != nil {
		return "", fmt.Errorf("failed to create request: %w", err)
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
	req.Header.Set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")

	resp, err := bs.client.Do(req)
	if err != nil {
		return "", fmt.Errorf("failed to fetch: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != 200 {
		return "", fmt.Errorf("HTTP %d: %s", resp.StatusCode, resp.Status)
	}

	body, err := io.ReadAll(io.LimitReader(resp.Body, 5*1024*1024)) // 5MB limit
	if err != nil {
		return "", fmt.Errorf("failed to read response: %w", err)
	}

	html := string(body)
	baseURL := fmt.Sprintf("%s://%s", parsed.Scheme, parsed.Host)

	// Clean the HTML
	cleaned := cleanHTML(html, baseURL, rawURL)

	// Cache it
	bs.mu.Lock()
	bs.cache[rawURL] = cachedPage{html: cleaned, fetchedAt: time.Now()}
	bs.mu.Unlock()

	return cleaned, nil
}

// ClearCache clears the page cache.
func (bs *BrowserService) ClearCache() {
	bs.mu.Lock()
	bs.cache = make(map[string]cachedPage)
	bs.mu.Unlock()
}

var (
	scriptRegex    = regexp.MustCompile(`(?is)<script[\s>].*?</script>`)
	noscriptRegex  = regexp.MustCompile(`(?is)<noscript[\s>].*?</noscript>`)
	commentRegex   = regexp.MustCompile(`(?s)<!--.*?-->`)
	onEventRegex   = regexp.MustCompile(`(?i)\s+on\w+="[^"]*"`)
	onEventRegex2  = regexp.MustCompile(`(?i)\s+on\w+='[^']*'`)
)

func cleanHTML(html string, baseURL string, pageURL string) string {
	// Remove scripts
	html = scriptRegex.ReplaceAllString(html, "")
	// Remove noscript (show noscript content would be better but complex)
	html = noscriptRegex.ReplaceAllString(html, "")
	// Remove HTML comments
	html = commentRegex.ReplaceAllString(html, "")
	// Remove inline event handlers
	html = onEventRegex.ReplaceAllString(html, "")
	html = onEventRegex2.ReplaceAllString(html, "")

	// Add a <base> tag so relative URLs resolve correctly
	baseTag := fmt.Sprintf(`<base href="%s" target="_blank">`, pageURL)
	if strings.Contains(strings.ToLower(html), "<head") {
		html = regexp.MustCompile(`(?i)(<head[^>]*>)`).ReplaceAllString(html, "${1}"+baseTag)
	} else if strings.Contains(strings.ToLower(html), "<html") {
		html = regexp.MustCompile(`(?i)(<html[^>]*>)`).ReplaceAllString(html, "${1}<head>"+baseTag+"</head>")
	} else {
		html = baseTag + html
	}

	// Inject dark mode override and link interception styles
	styleOverride := `<style>
		:root { color-scheme: light; }
		a[href] { cursor: pointer; color: #2563eb; }
	</style>`
	html = strings.Replace(html, "</head>", styleOverride+"</head>", 1)

	return html
}
