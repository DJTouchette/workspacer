// project_status — one call for the git state of every configured project.
//
// A standup opened with a per-repo shell-out: read the projects config, then
// `git status` / `git rev-list` in each one, several round trips deep, every
// time. The manager is a delegator whose turns are supposed to end in seconds,
// and that is the one read it makes constantly.
//
// Like respawn_with, this is a FACADE-SIDE COMPOSITION rather than a new
// capability: it calls config.get and git.status, both of which the caller's
// tier already holds, so nothing new is registered on the bus and its gate is
// DERIVED from its parts — it cannot be reachable where git.status is not.
// (git.* is in neither scoped tier's allowlist, so that is operator, and a
// view scout cannot enumerate the user's repositories through it.)
//
// The per-project calls run CONCURRENTLY. Sequentially, the tool would trade
// the manager's several round trips for one round trip that internally takes
// just as long, which is not the fix the friction described.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type projectStatusIn struct {
	hubArg
	Dirs []string `json:"dirs,omitempty" jsonschema:"specific project directories to report on; omit for every project in the workspacer config"`
}

// gitStatusResult is the shape git.status answers with (gitService.status /
// the brain's twin): the branch header's upstream/ahead/behind plus one entry
// per changed file.
type gitStatusResult struct {
	Branch   *string `json:"branch"`
	Upstream *string `json:"upstream"`
	Ahead    int     `json:"ahead"`
	Behind   int     `json:"behind"`
	Files    []struct {
		Status string `json:"status"`
		Path   string `json:"path"`
	} `json:"files"`
}

// projectRow is one project's line in the answer. Deliberately flat and small —
// this is read by a model composing a four-line digest, not by a UI.
type projectRow struct {
	Dir string `json:"dir"`
	// Branch is nil on a detached HEAD or an empty repo, which is meaningfully
	// different from an empty string.
	Branch   *string `json:"branch,omitempty"`
	Upstream *string `json:"upstream,omitempty"`
	// Unpushed is commits ahead of the upstream — the number a standup means by
	// "not pushed yet". Omitted entirely when there is no upstream, because 0
	// there would read as "nothing to push" when the truth is "nowhere to push".
	Unpushed *int   `json:"unpushed,omitempty"`
	Behind   *int   `json:"behind,omitempty"`
	Dirty    bool   `json:"dirty"`
	Changed  int    `json:"changedFiles"`
	Error    string `json:"error,omitempty"`
}

// addProjectStatusTool registers project_status when the tier may call both
// halves. Same derived gate as respawn_with.
func addProjectStatusTool(b *build) {
	const (
		configMethod = "config.get"
		gitMethod    = "git.status"
	)
	if !b.allowed(configMethod) || !b.allowed(gitMethod) {
		return
	}
	const name = "project_status"
	const desc = "Git state for every configured project in ONE call: branch, upstream, unpushed commits, behind count, and whether the tree is dirty. Replaces the per-repo shell-outs a standup needs. Pass dirs to limit it. A directory that is not a repo (or errors) comes back as a row with an error, never as a failed call."
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: gitMethod, Group: b.group})

	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in projectStatusIn) (*mcp.CallToolResult, any, error) {
			peer := in.takeHub()
			route := func(m string) string {
				if peer == "" {
					return m
				}
				return "hub:" + peer + "/" + m
			}

			dirs := trimmedNonEmpty(in.Dirs)
			if len(dirs) == 0 {
				raw, err := b.call(ctx, route(configMethod), nil)
				if err != nil {
					return toolError(fmt.Sprintf("project_status: could not read the config: %v", err))
				}
				dirs = configuredProjectDirs(raw)
				if len(dirs) == 0 {
					return toolError("project_status: no projects are configured (config `projects` is empty) and no dirs were given. Pass dirs explicitly, or add the repos in Settings.")
				}
			}

			rows := make([]projectRow, len(dirs))
			var wg sync.WaitGroup
			for i, dir := range dirs {
				wg.Add(1)
				go func(i int, dir string) {
					defer wg.Done()
					rows[i] = projectStatusFor(ctx, b, route(gitMethod), dir)
				}(i, dir)
			}
			wg.Wait()

			out, err := json.Marshal(map[string]any{"projects": rows})
			if err != nil {
				return toolError(fmt.Sprintf("project_status: could not render the answer: %v", err))
			}
			return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: string(out)}}}, nil, nil
		})
}

// projectStatusFor reports one project. A failure is a ROW, not an error: a
// standup over six repos must not lose the five that answered because the sixth
// is not a git checkout.
func projectStatusFor(ctx context.Context, b *build, method, dir string) projectRow {
	row := projectRow{Dir: dir}
	raw, err := b.call(ctx, method, map[string]string{"cwd": dir})
	if err != nil {
		row.Error = err.Error()
		return row
	}
	var st gitStatusResult
	if json.Unmarshal(raw, &st) != nil {
		row.Error = "git.status returned a shape this tool could not read"
		return row
	}
	row.Branch = st.Branch
	row.Changed = len(st.Files)
	row.Dirty = row.Changed > 0
	if st.Upstream != nil && strings.TrimSpace(*st.Upstream) != "" {
		up, ahead, behind := *st.Upstream, st.Ahead, st.Behind
		row.Upstream, row.Unpushed, row.Behind = &up, &ahead, &behind
	}
	return row
}

// configuredProjectDirs pulls the project directories out of a config document.
// `projects` is keyed BY directory, so the keys are the answer. Sorted, because
// the map iteration order is not stable and a status list that reshuffles
// between calls reads as churn.
func configuredProjectDirs(raw json.RawMessage) []string {
	var cfg struct {
		Projects map[string]json.RawMessage `json:"projects"`
	}
	if json.Unmarshal(raw, &cfg) != nil {
		return nil
	}
	dirs := make([]string, 0, len(cfg.Projects))
	for dir := range cfg.Projects {
		if strings.TrimSpace(dir) != "" {
			dirs = append(dirs, dir)
		}
	}
	sort.Strings(dirs)
	return dirs
}

func trimmedNonEmpty(in []string) []string {
	out := make([]string, 0, len(in))
	for _, v := range in {
		if strings.TrimSpace(v) != "" {
			out = append(out, v)
		}
	}
	return out
}
