// Package usage scans ~/.claude/projects/ for session JSONL transcripts and computes usage.
package usage

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

// SessionUsage represents usage data for a single Claude Code session.
type SessionUsage struct {
	SessionID        string  `json:"sessionId"`
	ProjectPath      string  `json:"projectPath"`
	ProjectName      string  `json:"projectName"`
	Model            string  `json:"model"`
	Turns            int     `json:"turns"`
	InputTokens      int64   `json:"inputTokens"`
	OutputTokens     int64   `json:"outputTokens"`
	CacheReadTokens  int64   `json:"cacheReadTokens"`
	CacheWriteTokens int64   `json:"cacheWriteTokens"`
	Cost             float64 `json:"cost"`
	LastActivity     string  `json:"lastActivity"`
}

// UsageSummary is the top-level aggregation of all session usage.
type UsageSummary struct {
	TotalSessions    int            `json:"totalSessions"`
	TotalCost        float64        `json:"totalCost"`
	TotalInputTokens int64          `json:"totalInputTokens"`
	TotalOutputTokens int64         `json:"totalOutputTokens"`
	TotalCacheRead   int64          `json:"totalCacheRead"`
	TotalCacheWrite  int64          `json:"totalCacheWrite"`
	ByProject        []ProjectUsage `json:"byProject"`
	RecentSessions   []SessionUsage `json:"recentSessions"`
}

// ProjectUsage aggregates usage across all sessions for a project.
type ProjectUsage struct {
	ProjectPath  string  `json:"projectPath"`
	ProjectName  string  `json:"projectName"`
	SessionCount int     `json:"sessionCount"`
	TotalCost    float64 `json:"totalCost"`
	TotalTokens  int64   `json:"totalTokens"`
}

// RollingWindowUsage represents usage within a rolling 5-hour window.
type RollingWindowUsage struct {
	TotalTokens      int64         `json:"totalTokens"`
	InputTokens      int64         `json:"inputTokens"`
	OutputTokens     int64         `json:"outputTokens"`
	CacheReadTokens  int64         `json:"cacheReadTokens"`
	CacheWriteTokens int64         `json:"cacheWriteTokens"`
	EstimatedCost    float64       `json:"estimatedCost"`
	WindowStart      string        `json:"windowStart"`
	WindowEnd        string        `json:"windowEnd"`
	HourlyBreakdown  [5]HourBucket `json:"hourlyBreakdown"`
	ActiveSessions   int           `json:"activeSessions"`
}

// HourBucket holds token/cost data for one hour within the rolling window.
type HourBucket struct {
	Hour   string  `json:"hour"`
	Tokens int64   `json:"tokens"`
	Cost   float64 `json:"cost"`
}

// cache stores the last scan result for 30 seconds.
var (
	cacheMu     sync.Mutex
	cachedResult *UsageSummary
	lastScan    time.Time
	cacheTTL    = 30 * time.Second

	windowCacheMu     sync.Mutex
	cachedWindowResult *RollingWindowUsage
	lastWindowScan    time.Time
)

// Scan reads ~/.claude/projects/ and computes usage. Results are cached for 30 seconds.
func Scan() (*UsageSummary, error) {
	cacheMu.Lock()
	if cachedResult != nil && time.Since(lastScan) < cacheTTL {
		result := cachedResult
		cacheMu.Unlock()
		return result, nil
	}
	cacheMu.Unlock()

	result, err := scan()
	if err != nil {
		return nil, err
	}

	cacheMu.Lock()
	cachedResult = result
	lastScan = time.Now()
	cacheMu.Unlock()

	return result, nil
}

func scan() (*UsageSummary, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, err
	}
	projectsDir := filepath.Join(home, ".claude", "projects")

	entries, err := os.ReadDir(projectsDir)
	if err != nil {
		// If directory doesn't exist, return empty summary
		if os.IsNotExist(err) {
			return &UsageSummary{}, nil
		}
		return nil, err
	}

	var allSessions []SessionUsage
	projectMap := make(map[string]*ProjectUsage)

	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		dirName := entry.Name()
		projectPath := dirNameToDisplay(dirName)
		projectNameStr := projectNameFromDir(dirName)

		projDir := filepath.Join(projectsDir, dirName)
		files, err := os.ReadDir(projDir)
		if err != nil {
			continue
		}

		for _, f := range files {
			if f.IsDir() || !strings.HasSuffix(f.Name(), ".jsonl") {
				continue
			}

			sessionID := strings.TrimSuffix(f.Name(), ".jsonl")
			filePath := filepath.Join(projDir, f.Name())

			info, err := f.Info()
			if err != nil {
				continue
			}
			modTime := info.ModTime().UTC().Format(time.RFC3339)

			su := parseSessionFile(filePath, sessionID, projectPath, projectNameStr, modTime)
			if su.Turns == 0 && su.InputTokens == 0 && su.OutputTokens == 0 {
				continue
			}

			allSessions = append(allSessions, su)

			pu, ok := projectMap[dirName]
			if !ok {
				pu = &ProjectUsage{
					ProjectPath: projectPath,
					ProjectName: projectNameStr,
				}
				projectMap[dirName] = pu
			}
			pu.SessionCount++
			pu.TotalCost += su.Cost
			pu.TotalTokens += su.InputTokens + su.OutputTokens + su.CacheReadTokens + su.CacheWriteTokens
		}
	}

	// Sort all sessions by mod time descending
	sort.Slice(allSessions, func(i, j int) bool {
		return allSessions[i].LastActivity > allSessions[j].LastActivity
	})

	// Build summary
	summary := &UsageSummary{}
	summary.TotalSessions = len(allSessions)
	for _, s := range allSessions {
		summary.TotalCost += s.Cost
		summary.TotalInputTokens += s.InputTokens
		summary.TotalOutputTokens += s.OutputTokens
		summary.TotalCacheRead += s.CacheReadTokens
		summary.TotalCacheWrite += s.CacheWriteTokens
	}

	// Recent sessions: top 20
	limit := 20
	if len(allSessions) < limit {
		limit = len(allSessions)
	}
	summary.RecentSessions = allSessions[:limit]

	// By project sorted by cost descending
	var byProject []ProjectUsage
	for _, pu := range projectMap {
		byProject = append(byProject, *pu)
	}
	sort.Slice(byProject, func(i, j int) bool {
		return byProject[i].TotalCost > byProject[j].TotalCost
	})
	summary.ByProject = byProject

	return summary, nil
}

// jsonlEntry represents a single line in the JSONL transcript.
type jsonlEntry struct {
	Type      string `json:"type"`
	Timestamp string `json:"timestamp"`
	Message   *struct {
		Model string `json:"model"`
		Usage *struct {
			InputTokens              int64 `json:"input_tokens"`
			OutputTokens             int64 `json:"output_tokens"`
			CacheReadInputTokens     int64 `json:"cache_read_input_tokens"`
			CacheCreationInputTokens int64 `json:"cache_creation_input_tokens"`
		} `json:"usage"`
	} `json:"message"`
}

func parseSessionFile(path, sessionID, projectPath, projectName, modTime string) SessionUsage {
	su := SessionUsage{
		SessionID:    sessionID,
		ProjectPath:  projectPath,
		ProjectName:  projectName,
		LastActivity: modTime,
	}

	f, err := os.Open(path)
	if err != nil {
		return su
	}
	defer f.Close()

	scanner := bufio.NewScanner(f)
	// Allow large lines (JSONL entries can be big)
	scanner.Buffer(make([]byte, 0, 64*1024), 2*1024*1024)

	for scanner.Scan() {
		var entry jsonlEntry
		if err := json.Unmarshal(scanner.Bytes(), &entry); err != nil {
			continue
		}
		if entry.Type != "assistant" || entry.Message == nil || entry.Message.Usage == nil {
			continue
		}

		su.Turns++
		usage := entry.Message.Usage
		if su.Model == "" && entry.Message.Model != "" {
			su.Model = entry.Message.Model
		}

		su.InputTokens += usage.InputTokens
		su.OutputTokens += usage.OutputTokens
		su.CacheReadTokens += usage.CacheReadInputTokens
		su.CacheWriteTokens += usage.CacheCreationInputTokens
	}

	su.Cost = calculateCost(su.Model, su.InputTokens, su.OutputTokens, su.CacheReadTokens, su.CacheWriteTokens)
	return su
}

func calculateCost(model string, inputTokens, outputTokens, cacheRead, cacheWrite int64) float64 {
	m := strings.ToLower(model)

	var inputPer1M, outputPer1M, cacheReadPer1M, cacheWritePer1M float64

	switch {
	case strings.Contains(m, "opus"):
		inputPer1M = 15.0
		outputPer1M = 75.0
		cacheReadPer1M = 1.50
		cacheWritePer1M = 18.75
	case strings.Contains(m, "haiku"):
		inputPer1M = 0.25
		outputPer1M = 1.25
		cacheReadPer1M = 0.025
		cacheWritePer1M = 0.3125
	default: // sonnet
		inputPer1M = 3.0
		outputPer1M = 15.0
		cacheReadPer1M = 0.30
		cacheWritePer1M = 3.75
	}

	cost := float64(inputTokens)*inputPer1M/1_000_000 +
		float64(outputTokens)*outputPer1M/1_000_000 +
		float64(cacheRead)*cacheReadPer1M/1_000_000 +
		float64(cacheWrite)*cacheWritePer1M/1_000_000

	return cost
}

// ScanRollingWindow scans all JSONL transcripts and returns usage within the last 5 hours.
// Results are cached for 30 seconds.
func ScanRollingWindow() (*RollingWindowUsage, error) {
	windowCacheMu.Lock()
	if cachedWindowResult != nil && time.Since(lastWindowScan) < cacheTTL {
		result := cachedWindowResult
		windowCacheMu.Unlock()
		return result, nil
	}
	windowCacheMu.Unlock()

	result, err := scanRollingWindow()
	if err != nil {
		return nil, err
	}

	windowCacheMu.Lock()
	cachedWindowResult = result
	lastWindowScan = time.Now()
	windowCacheMu.Unlock()

	return result, nil
}

func scanRollingWindow() (*RollingWindowUsage, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, err
	}
	projectsDir := filepath.Join(home, ".claude", "projects")

	now := time.Now().UTC()
	windowStart := now.Add(-5 * time.Hour)

	result := &RollingWindowUsage{
		WindowStart: windowStart.Format(time.RFC3339),
		WindowEnd:   now.Format(time.RFC3339),
	}

	// Initialize hourly buckets
	for i := 0; i < 5; i++ {
		bucketTime := now.Add(time.Duration(-(4 - i)) * time.Hour)
		result.HourlyBreakdown[i] = HourBucket{
			Hour: fmt.Sprintf("%02d:00", bucketTime.Hour()),
		}
	}

	entries, err := os.ReadDir(projectsDir)
	if err != nil {
		if os.IsNotExist(err) {
			return result, nil
		}
		return nil, err
	}

	activeSessionSet := make(map[string]bool)
	var model string

	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		projDir := filepath.Join(projectsDir, entry.Name())
		files, err := os.ReadDir(projDir)
		if err != nil {
			continue
		}

		for _, f := range files {
			if f.IsDir() || !strings.HasSuffix(f.Name(), ".jsonl") {
				continue
			}

			// Quick filter: skip files not modified in the last 5 hours
			info, err := f.Info()
			if err != nil {
				continue
			}
			if info.ModTime().Before(windowStart) {
				continue
			}

			filePath := filepath.Join(projDir, f.Name())
			sessionID := strings.TrimSuffix(f.Name(), ".jsonl")

			sessionActive := false
			fp, err := os.Open(filePath)
			if err != nil {
				continue
			}

			scanner := bufio.NewScanner(fp)
			scanner.Buffer(make([]byte, 0, 64*1024), 2*1024*1024)

			for scanner.Scan() {
				var e jsonlEntry
				if err := json.Unmarshal(scanner.Bytes(), &e); err != nil {
					continue
				}
				if e.Type != "assistant" || e.Message == nil || e.Message.Usage == nil {
					continue
				}

				// Parse the entry timestamp
				ts, err := time.Parse(time.RFC3339Nano, e.Timestamp)
				if err != nil {
					// Try RFC3339 without nanos
					ts, err = time.Parse(time.RFC3339, e.Timestamp)
					if err != nil {
						continue
					}
				}

				if ts.Before(windowStart) {
					continue
				}

				usage := e.Message.Usage
				tokens := usage.InputTokens + usage.OutputTokens + usage.CacheReadInputTokens + usage.CacheCreationInputTokens

				result.InputTokens += usage.InputTokens
				result.OutputTokens += usage.OutputTokens
				result.CacheReadTokens += usage.CacheReadInputTokens
				result.CacheWriteTokens += usage.CacheCreationInputTokens
				result.TotalTokens += tokens

				if model == "" && e.Message.Model != "" {
					model = e.Message.Model
				}

				sessionActive = true

				// Assign to hourly bucket
				hoursAgo := now.Sub(ts).Hours()
				bucketIdx := 4 - int(hoursAgo)
				if bucketIdx < 0 {
					bucketIdx = 0
				}
				if bucketIdx > 4 {
					bucketIdx = 4
				}
				result.HourlyBreakdown[bucketIdx].Tokens += tokens
			}
			fp.Close()

			if sessionActive {
				activeSessionSet[sessionID] = true
			}
		}
	}

	result.ActiveSessions = len(activeSessionSet)
	result.EstimatedCost = calculateCost(model, result.InputTokens, result.OutputTokens, result.CacheReadTokens, result.CacheWriteTokens)

	// Compute per-bucket cost (proportional to tokens)
	if result.TotalTokens > 0 {
		for i := range result.HourlyBreakdown {
			fraction := float64(result.HourlyBreakdown[i].Tokens) / float64(result.TotalTokens)
			result.HourlyBreakdown[i].Cost = result.EstimatedCost * fraction
		}
	}

	return result, nil
}

// dirNameToDisplay converts a directory name like "-home-djtouchette-Work-foo"
// into a display path. If it starts with "-home-", strip that prefix for cleaner display.
func dirNameToDisplay(dirName string) string {
	if dirName == "" {
		return dirName
	}
	// Remove leading dash to get "home-djtouchette-Work-foo"
	s := strings.TrimPrefix(dirName, "-")
	// Replace dashes with slashes: "home/djtouchette/Work/foo"
	// Note: this is ambiguous for project names with dashes, but
	// it provides a best-effort display path.
	s = "/" + strings.ReplaceAll(s, "-", "/")
	return s
}

// projectNameFromDir extracts a display-friendly project name from the dir name.
// Uses the portion after the last recognizable path separator pattern.
func projectNameFromDir(dirName string) string {
	// The dir name is like "-home-djtouchette-Work-worky-workspacer"
	// We just take the raw dirname and show it; users will recognize it.
	// Strip the leading "-home-username-" part for brevity.
	s := strings.TrimPrefix(dirName, "-")
	parts := strings.SplitN(s, "-", 4) // home, user, Work, rest...
	if len(parts) >= 4 {
		return parts[3] // e.g. "worky-workspacer" or "quote-pilot"
	}
	if len(parts) >= 3 {
		return parts[2]
	}
	return dirName
}
