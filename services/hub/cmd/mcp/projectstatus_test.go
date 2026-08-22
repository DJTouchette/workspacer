package main

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestConfiguredProjectDirsAreTheKeysAndAreSorted(t *testing.T) {
	cfg := `{"projects":{"/w/zeta":{"delivery":"pr"},"/w/alpha":{},"  ":{}}}`
	got := configuredProjectDirs(json.RawMessage(cfg))
	// Sorted, because map iteration order is not stable and a status list that
	// reshuffles between calls reads as churn.
	if len(got) != 2 || got[0] != "/w/alpha" || got[1] != "/w/zeta" {
		t.Errorf("configuredProjectDirs = %v", got)
	}
	if len(configuredProjectDirs(json.RawMessage(`not json`))) != 0 {
		t.Error("an unreadable config must yield no dirs, not a panic")
	}
	if len(configuredProjectDirs(json.RawMessage(`{}`))) != 0 {
		t.Error("a config with no projects must yield no dirs")
	}
}

// statusHub answers config.get and git.status per directory, recording the
// calls so the test can assert on fan-out as well as on the answer.
type statusHub struct {
	mu      sync.Mutex
	cwds    []string
	config  string
	perDir  map[string]string
	errDirs map[string]bool
	// gate blocks every git.status until closed, so concurrency is provable
	// rather than assumed.
	gate chan struct{}
}

func (h *statusHub) Call(_ context.Context, method string, params any) (json.RawMessage, error) {
	if method == "config.get" {
		return json.RawMessage(h.config), nil
	}
	raw, _ := json.Marshal(params)
	var p struct {
		Cwd string `json:"cwd"`
	}
	_ = json.Unmarshal(raw, &p)
	h.mu.Lock()
	h.cwds = append(h.cwds, p.Cwd)
	n := len(h.cwds)
	h.mu.Unlock()
	if h.gate != nil {
		// Every caller must arrive before ANY is released; a sequential
		// implementation deadlocks here instead of quietly being slow.
		if n == cap(h.gate) {
			close(h.gate)
		}
		<-h.gate
	}
	if h.errDirs[p.Cwd] {
		return nil, errFake
	}
	if body, ok := h.perDir[p.Cwd]; ok {
		return json.RawMessage(body), nil
	}
	return json.RawMessage(`{"branch":"master","files":[]}`), nil
}

func callProjectStatus(t *testing.T, hub *statusHub, args map[string]any) (*mcp.CallToolResult, []projectRow) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	srv := mcp.NewServer(&mcp.Implementation{Name: "workspacer-test", Version: "v1"}, nil)
	b := &build{
		s: srv, scope: authtoken.ScopeOperator, allow: authtoken.ScopeOperator.Methods(),
		caller: func(ctx context.Context, method string, params any) (json.RawMessage, error) {
			return hub.Call(ctx, method, params)
		},
	}
	addProjectStatusTool(b)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := srv.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer cs.Close()

	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "project_status", Arguments: args})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	if res.IsError {
		return res, nil
	}
	var out struct {
		Projects []projectRow `json:"projects"`
	}
	if err := json.Unmarshal([]byte(resultText(res)), &out); err != nil {
		t.Fatalf("result is not the expected shape (%v): %s", err, resultText(res))
	}
	return res, out.Projects
}

func TestProjectStatusReportsBranchAheadBehindAndDirt(t *testing.T) {
	hub := &statusHub{
		config: `{"projects":{"/w/alpha":{},"/w/beta":{}}}`,
		perDir: map[string]string{
			"/w/alpha": `{"branch":"wks/fix","upstream":"origin/wks/fix","ahead":3,"behind":1,
				"files":[{"status":"M","path":"a.ts"},{"status":"??","path":"b.ts"}]}`,
			"/w/beta": `{"branch":"master","upstream":null,"ahead":0,"behind":0,"files":[]}`,
		},
	}
	_, rows := callProjectStatus(t, hub, map[string]any{})
	if len(rows) != 2 {
		t.Fatalf("want 2 rows, got %d", len(rows))
	}

	alpha := rows[0]
	if alpha.Dir != "/w/alpha" || alpha.Branch == nil || *alpha.Branch != "wks/fix" {
		t.Errorf("alpha branch: %+v", alpha)
	}
	if alpha.Unpushed == nil || *alpha.Unpushed != 3 {
		t.Errorf("alpha unpushed: %+v", alpha)
	}
	if alpha.Behind == nil || *alpha.Behind != 1 {
		t.Errorf("alpha behind: %+v", alpha)
	}
	if !alpha.Dirty || alpha.Changed != 2 {
		t.Errorf("alpha dirt: %+v", alpha)
	}

	beta := rows[1]
	if beta.Dirty || beta.Changed != 0 {
		t.Errorf("beta should be clean: %+v", beta)
	}
	// No upstream: unpushed is OMITTED, not 0. Zero would read as "nothing to
	// push" when the truth is "nowhere to push".
	if beta.Unpushed != nil || beta.Upstream != nil {
		t.Errorf("beta has no upstream, so unpushed must be absent: %+v", beta)
	}
}

func TestProjectStatusReportsAFailedRepoAsAROW(t *testing.T) {
	hub := &statusHub{
		config:  `{"projects":{"/w/alpha":{},"/w/notarepo":{}}}`,
		errDirs: map[string]bool{"/w/notarepo": true},
	}
	_, rows := callProjectStatus(t, hub, map[string]any{})
	// A standup over six repos must not lose the five that answered because the
	// sixth is not a git checkout.
	if len(rows) != 2 {
		t.Fatalf("want both rows, got %d", len(rows))
	}
	if rows[0].Error != "" {
		t.Errorf("the healthy repo should have no error: %+v", rows[0])
	}
	if rows[1].Error == "" {
		t.Errorf("the broken repo should carry its error: %+v", rows[1])
	}
}

func TestProjectStatusHonoursExplicitDirsAndSkipsTheConfig(t *testing.T) {
	hub := &statusHub{config: `{"projects":{"/w/alpha":{},"/w/beta":{}}}`}
	_, rows := callProjectStatus(t, hub, map[string]any{"dirs": []string{"/w/gamma", "  "}})
	if len(rows) != 1 || rows[0].Dir != "/w/gamma" {
		t.Fatalf("explicit dirs should win (and blanks be dropped): %+v", rows)
	}
}

func TestProjectStatusRefusesWhenThereIsNothingToReportOn(t *testing.T) {
	hub := &statusHub{config: `{"projects":{}}`}
	res, _ := callProjectStatus(t, hub, map[string]any{})
	if !res.IsError || !strings.Contains(resultText(res), "no projects are configured") {
		t.Errorf("want an explanatory refusal, got: %s", resultText(res))
	}
}

// The whole point is replacing SEVERAL round trips with one, so the per-project
// calls must overlap. The gate releases only once every project has arrived: a
// sequential implementation blocks forever and this fails on the context
// deadline rather than passing slowly.
func TestProjectStatusFansOutConcurrently(t *testing.T) {
	hub := &statusHub{
		config: `{"projects":{"/w/a":{},"/w/b":{},"/w/c":{},"/w/d":{}}}`,
		gate:   make(chan struct{}, 4),
	}
	_, rows := callProjectStatus(t, hub, map[string]any{})
	if len(rows) != 4 {
		t.Fatalf("want 4 rows, got %d", len(rows))
	}
}
