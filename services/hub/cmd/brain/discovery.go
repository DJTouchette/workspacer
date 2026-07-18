package main

// claude.sessionsForDir — discover existing Claude Code sessions for a directory
// (the resume picker). Reads ~/.claude/projects/<encoded-cwd>/*.jsonl, peeking at
// each transcript's head for a name/first-message. Ports claudeSessionList.ts;
// the path encoding must match claudemon's encoded_cwd.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"unicode/utf8"
)

type claudeSessionSummary struct {
	SessionID string `json:"sessionId"`
	Timestamp string `json:"timestamp"`
	Summary   string `json:"summary"`
}

var (
	reTrailingSep = regexp.MustCompile(`[/\\]+$`)
	reSepColon    = regexp.MustCompile(`[/\\:]`)
)

// encodeDirName names the per-project transcript folder like the Claude CLI:
// drop a trailing separator, then map every '/', '\' and ':' to '-' with NO
// stripping (so '/foo/bar' → '-foo-bar'). Must match claudemon's encoded_cwd.
func encodeDirName(dir string) string {
	return reSepColon.ReplaceAllString(reTrailingSep.ReplaceAllString(dir, ""), "-")
}

func listClaudeSessionsForDir(cwd string) []claudeSessionSummary {
	out := []claudeSessionSummary{}
	home, err := os.UserHomeDir()
	if err != nil {
		return out
	}
	projectDir := filepath.Join(home, ".claude", "projects", encodeDirName(cwd))
	entries, err := os.ReadDir(projectDir)
	if err != nil {
		return out
	}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || !strings.HasSuffix(name, ".jsonl") {
			continue
		}
		sessionID := strings.TrimSuffix(name, ".jsonl")
		if strings.HasPrefix(sessionID, "agent-") { // skip subagent transcripts
			continue
		}
		full := filepath.Join(projectDir, name)
		st, err := os.Stat(full)
		if err != nil {
			continue
		}
		summary := scanTranscriptSummary(full)
		if summary == "" {
			summary = sessionID
		}
		out = append(out, claudeSessionSummary{
			SessionID: sessionID,
			Timestamp: st.ModTime().UTC().Format("2006-01-02T15:04:05.000Z"),
			Summary:   summary,
		})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Timestamp > out[j].Timestamp })
	if len(out) > 20 {
		out = out[:20]
	}
	return out
}

// scanTranscriptSummary reads the head of a transcript and returns its summary
// line (a `summary` entry wins; otherwise the first user message), clipped to
// 100 chars — matching the app's 8KB peek.
func scanTranscriptSummary(path string) string {
	f, err := os.Open(path)
	if err != nil {
		return ""
	}
	defer f.Close()
	buf := make([]byte, 8192)
	n, _ := f.Read(buf)
	summary := ""
	for _, line := range strings.Split(string(buf[:n]), "\n") {
		if strings.TrimSpace(line) == "" {
			continue
		}
		var entry map[string]any
		if json.Unmarshal([]byte(line), &entry) != nil {
			continue
		}
		if entry["type"] == "summary" {
			if s, ok := entry["summary"].(string); ok && s != "" {
				return clip(s, 100)
			}
		}
		if summary == "" && entry["type"] == "user" {
			if content := userMessageText(entry["message"]); content != "" {
				summary = strings.ReplaceAll(clip(content, 100), "\n", " ")
			}
		}
	}
	return summary
}

func userMessageText(msg any) string {
	m, ok := msg.(map[string]any)
	if !ok {
		return ""
	}
	switch c := m["content"].(type) {
	case string:
		return c
	case []any:
		var parts []string
		for _, b := range c {
			if bm, ok := b.(map[string]any); ok && bm["type"] == "text" {
				if t, ok := bm["text"].(string); ok {
					parts = append(parts, t)
				}
			}
		}
		return strings.Join(parts, "\n")
	}
	return ""
}

func clip(s string, n int) string {
	if utf8.RuneCountInString(s) > n {
		runes := []rune(s)
		return string(runes[:n])
	}
	return s
}
