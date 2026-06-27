package main

// search.project — project-wide text search, shelling out to ripgrep (`rg
// --json`), which is fast and gitignore-aware. Ports searchService.ts. The app
// bundles @vscode/ripgrep; headless we use the host's `rg` on PATH.

import (
	"context"
	"encoding/json"
	"fmt"
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
	maxResults := opts.MaxResults
	if maxResults <= 0 {
		maxResults = searchMaxResults
	}

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
	out, err := cmd.Output()
	if err != nil {
		// rg exits 1 on "no matches" — success here. >=2 is a real error; a
		// missing binary is reported plainly.
		if ee, ok := err.(*exec.ExitError); ok {
			if ee.ExitCode() != 1 {
				return res, fmt.Errorf("ripgrep failed (exit %d)", ee.ExitCode())
			}
		} else {
			return res, fmt.Errorf("ripgrep not runnable (is `rg` on PATH?): %w", err)
		}
	}

	byFile := map[string]*searchFileResult{}
	var order []string
	total := 0
	for _, line := range strings.Split(string(out), "\n") {
		if line == "" {
			continue
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
			continue
		}
		if total >= maxResults {
			res.Truncated = true
			break
		}
		rel := msg.Data.Path.Text
		if rel == "" {
			continue
		}
		abs := filepath.Join(opts.Cwd, rel) // rg reports paths relative to cwd
		col := 1
		if len(msg.Data.Submatches) > 0 {
			col = msg.Data.Submatches[0].Start + 1
		}
		m := searchMatch{
			Line:   msg.Data.LineNumber,
			Column: col,
			Text:   clip(strings.TrimSpace(strings.TrimRight(msg.Data.Lines.Text, "\r\n")), searchMaxTextLen),
		}
		bucket := byFile[abs]
		if bucket == nil {
			bucket = &searchFileResult{File: abs}
			byFile[abs] = bucket
			order = append(order, abs)
		}
		bucket.Matches = append(bucket.Matches, m)
		total++
	}
	for _, k := range order {
		res.Results = append(res.Results, *byFile[k])
	}
	return res, nil
}
