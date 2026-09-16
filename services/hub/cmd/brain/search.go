package main

// search.project — project-wide text search, shelling out to ripgrep (`rg
// --json`), which is fast and gitignore-aware. Ports searchService.ts. The app
// bundles @vscode/ripgrep; headless we use the host's `rg` on PATH.

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

const (
	searchTimeout    = 15 * time.Second
	searchMaxResults = 500
	searchMaxTextLen = 300
)

type searchOpts struct {
	Query         string `json:"query"`
	Cwd           string `json:"cwd"`
	CaseSensitive bool   `json:"caseSensitive"`
	WholeWord     bool   `json:"wholeWord"`
	Regex         bool   `json:"regex"`
	MaxResults    int    `json:"maxResults"`
}

// effectiveMaxResults resolves the caller's cap. A NON-POSITIVE value means
// "unset", not "return nothing".
//
// Pulled out of searchProject to be testable without a `rg` binary, and because
// it is a twin: searchService.ts wrote `opts.maxResults ?? DEFAULT_MAX_RESULTS`,
// and `??` only replaces null/undefined — so `maxResults: 0` was a literal cap
// of zero there (an empty result list flagged truncated:true) and the whole 500
// here. search.project is registered by both providers, so the same request got
// opposite answers depending on which one answered; under the shipping
// catalog-delegation layout that is this copy.
func effectiveMaxResults(requested int) int {
	if requested <= 0 {
		return searchMaxResults
	}
	return requested
}

type searchMatch struct {
	Line   int    `json:"line"`
	Column int    `json:"column"`
	Text   string `json:"text"`
}
type searchFileResult struct {
	File    string        `json:"file"`
	Matches []searchMatch `json:"matches"`
}
type searchProjectResult struct {
	Results   []searchFileResult `json:"results"`
	Truncated bool               `json:"truncated"`
}

func searchProject(ctx context.Context, opts searchOpts) (searchProjectResult, error) {
	res := searchProjectResult{Results: []searchFileResult{}}
	if opts.Query == "" {
		return res, nil
	}
	maxResults := effectiveMaxResults(opts.MaxResults)

	args := []string{"--json", "--line-number", "--column"}
	if opts.CaseSensitive {
		args = append(args, "-s")
	} else {
		args = append(args, "--smart-case")
	}
	if opts.WholeWord {
		args = append(args, "-w")
	}
	if !opts.Regex {
		args = append(args, "-F")
	}
	args = append(args, "--", opts.Query, ".") // the trailing '.' is required (else rg reads stdin)

	cctx, cancel := context.WithTimeout(ctx, searchTimeout)
	defer cancel()
	cmd := exec.CommandContext(cctx, "rg", args...)
	cmd.Dir = opts.Cwd

	// Streamed, not buffered. A broad query ("e", ".") over a large checkout
	// makes rg emit hundreds of megabytes of JSON — 60 MB on this repo alone —
	// and buffering it whole (then copying it again to parse) was an unbounded
	// memory spike in a long-lived daemon, all to return at most maxResults.
	// Reading line by line lets us stop, and kill rg, the moment we have enough.
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return res, fmt.Errorf("ripgrep stdout: %w", err)
	}
	if err := cmd.Start(); err != nil {
		return res, fmt.Errorf("ripgrep not runnable (is `rg` on PATH?): %w", err)
	}

	col := newRipgrepCollector(opts.Cwd, maxResults)
	// A Reader, not a Scanner: rg emits one JSON object per match, and a match
	// in a minified bundle is a single line far past Scanner's 64 KB limit.
	reader := bufio.NewReader(stdout)
	stoppedEarly := false
	for {
		line, rerr := reader.ReadString('\n')
		if line != "" && col.addLine(strings.TrimRight(line, "\r\n")) {
			stoppedEarly = true
			cancel() // we have enough; don't let rg keep searching
			break
		}
		if rerr != nil {
			break
		}
	}
	// Drain anything already written so Wait can't block on a full pipe.
	_, _ = io.Copy(io.Discard, stdout)
	werr := cmd.Wait()

	if !stoppedEarly && werr != nil {
		if errors.Is(cctx.Err(), context.DeadlineExceeded) {
			return res, fmt.Errorf("ripgrep timed out after %s", searchTimeout)
		}
		// rg exits 1 on "no matches" — success here. >=2 is a real error; a
		// missing binary is reported plainly.
		var ee *exec.ExitError
		if errors.As(werr, &ee) {
			if ee.ExitCode() != 1 {
				return res, fmt.Errorf("ripgrep failed (exit %d)", ee.ExitCode())
			}
		} else {
			return res, fmt.Errorf("ripgrep not runnable (is `rg` on PATH?): %w", werr)
		}
	}

	return col.result(), nil
}

// ripgrepCollector groups `rg --json` match lines into per-file results,
// stopping once maxResults is reached. Fed one line at a time so the caller can
// stream rg's output and kill it early instead of buffering the whole thing.
type ripgrepCollector struct {
	cwd        string
	maxResults int
	byFile     map[string]*searchFileResult
	order      []string
	total      int
	truncated  bool
}

func newRipgrepCollector(cwd string, maxResults int) *ripgrepCollector {
	return &ripgrepCollector{
		cwd:        cwd,
		maxResults: maxResults,
		byFile:     map[string]*searchFileResult{},
	}
}

// addLine folds one `rg --json` line in. Returns true once the result cap is
// reached, meaning the caller should stop reading. Every submatch on a line
// becomes its own result (a line with N occurrences yields N columns) — keeping
// only the first silently dropped the rest and reported one match where there
// were several.
func (c *ripgrepCollector) addLine(line string) bool {
	if line == "" {
		return false
	}
	var msg struct {
		Type string `json:"type"`
		Data struct {
			Path       struct{ Text string } `json:"path"`
			Lines      struct{ Text string } `json:"lines"`
			LineNumber int                   `json:"line_number"`
			Submatches []struct {
				Start int `json:"start"`
			} `json:"submatches"`
		} `json:"data"`
	}
	if json.Unmarshal([]byte(line), &msg) != nil || msg.Type != "match" {
		return false
	}
	rel := msg.Data.Path.Text
	if rel == "" {
		return false
	}
	abs := filepath.Join(c.cwd, rel) // rg reports paths relative to cwd
	// asciiWhitespace, not strings.TrimSpace: search.project is answered by
	// whichever provider is registered (the brain by default), and JS `.trim()`
	// and Go's TrimSpace do not agree on U+FEFF or U+0085 — so a matching line
	// beginning with a BOM came back with different `text` from each. Same
	// literal set as normalizeCwd's, and as searchService.ts's TEXT_TRIM.
	text := clip(strings.Trim(strings.TrimRight(msg.Data.Lines.Text, "\r\n"), asciiWhitespace), searchMaxTextLen)
	// One column per submatch. rg still emits a match message even with an
	// empty submatch list (rare), so fall back to column 1 rather than drop
	// the whole line.
	cols := make([]int, 0, len(msg.Data.Submatches))
	for _, sm := range msg.Data.Submatches {
		cols = append(cols, sm.Start+1)
	}
	if len(cols) == 0 {
		cols = []int{1}
	}
	for _, col := range cols {
		if c.total >= c.maxResults {
			c.truncated = true
			return true
		}
		bucket := c.byFile[abs]
		if bucket == nil {
			bucket = &searchFileResult{File: abs}
			c.byFile[abs] = bucket
			c.order = append(c.order, abs)
		}
		bucket.Matches = append(bucket.Matches, searchMatch{
			Line:   msg.Data.LineNumber,
			Column: col,
			Text:   text,
		})
		c.total++
	}
	return false
}

func (c *ripgrepCollector) result() searchProjectResult {
	res := searchProjectResult{Results: []searchFileResult{}, Truncated: c.truncated}
	for _, k := range c.order {
		res.Results = append(res.Results, *c.byFile[k])
	}
	return res
}

// parseRipgrepJSON turns a whole `rg --json` output buffer into grouped
// per-file results. The streaming path in searchProject feeds the collector
// directly; this is the unit-testable entry point that doesn't need a live
// ripgrep.
func parseRipgrepJSON(out []byte, cwd string, maxResults int) searchProjectResult {
	c := newRipgrepCollector(cwd, maxResults)
	for _, line := range strings.Split(string(out), "\n") {
		if c.addLine(line) {
			break
		}
	}
	return c.result()
}
