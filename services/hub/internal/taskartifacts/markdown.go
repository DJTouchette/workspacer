package taskartifacts

import (
	"fmt"
	"net/url"
	"path"
	"regexp"
	"strings"
)

var markdownImage = regexp.MustCompile(`!\[[^\]\r\n]*\]\(([^)\r\n]*)\)`)
var htmlImage = regexp.MustCompile(`(?i)<\s*(img|picture|source|iframe)\b`)

// Reports remain byte-for-byte copies. V1 requires images to be selected
// manifest-relative files; it never fetches an image URL or treats a producer
// absolute path as an opening destination. Unsupported Markdown image syntax
// fails explicitly instead of producing a report with missing dependencies.
func validateReportImages(name string, data []byte, entries []Entry) error {
	if strings.ToLower(path.Ext(name)) != ".md" {
		return nil
	}
	text := string(data)
	if htmlImage.MatchString(text) {
		return fmt.Errorf("report %s requires portable Markdown image references, not HTML embeds", name)
	}
	matches := markdownImage.FindAllStringSubmatch(text, MaxFiles*2+1)
	if len(matches) > MaxFiles*2 {
		return fmt.Errorf("report image reference count exceeds task limit")
	}
	if strings.Count(text, "![") != len(matches) {
		return fmt.Errorf("report %s uses unsupported image reference syntax", name)
	}
	images := map[string]bool{}
	for _, e := range entries {
		if e.Kind == "image" {
			images[e.Name] = true
		}
	}
	for _, match := range matches {
		destination := strings.TrimSpace(match[1])
		if strings.HasPrefix(destination, "<") && strings.HasSuffix(destination, ">") {
			destination = destination[1 : len(destination)-1]
		}
		u, err := url.Parse(destination)
		if err != nil || u.IsAbs() || u.Host != "" || strings.HasPrefix(u.Path, "/") || strings.Contains(u.Path, "\\") || u.RawQuery != "" {
			return fmt.Errorf("report %s image must name a selected relative artifact; no URL fetch is performed", name)
		}
		target := path.Clean(path.Join(path.Dir(name), u.Path))
		if target == ".." || strings.HasPrefix(target, "../") || !images[target] {
			return fmt.Errorf("report %s references an image not selected in its artifact manifest", name)
		}
	}
	return nil
}
