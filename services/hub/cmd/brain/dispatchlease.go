package main

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"os"
	"path/filepath"
	"time"
)

// Preparation allocates only on this host. A lease is scoped to the authenticated
// origin credential and its unguessable dispatch id; claiming it is irreversible.
type dispatchLease struct {
	Handoff  *handoffReceiptSelector `json:"handoff,omitempty"`
	Owner    string                  `json:"owner"`
	Repo     string                  `json:"repo"`
	Cwd      string                  `json:"cwd"`
	Provider string                  `json:"provider"`
	Worktree bool                    `json:"worktree"`
	Branch   string                  `json:"branch,omitempty"`
	Expires  int64                   `json:"expires"`
	Claimed  bool                    `json:"claimed"`
}

type persistedRemoteDispatch struct {
	ID      string          `json:"id"`
	Session string          `json:"session"`
	Seq     int64           `json:"seq"`
	Last    *dispatchUpdate `json:"last,omitempty"`
	Lease   *dispatchLease  `json:"lease,omitempty"`
	AckedAt int64           `json:"ackedAt,omitempty"`
}

func (s *remoteDispatchStore) persistLocked() error {
	if s.file == "" {
		return nil
	}
	rows := make([]persistedRemoteDispatch, 0, len(s.m))
	for id, d := range s.m {
		rows = append(rows, persistedRemoteDispatch{id, d.sessionID, d.seq, d.last, d.lease, d.acknowledgedAt})
	}
	b, err := json.Marshal(rows)
	if err != nil {
		return err
	}
	return writeFileAtomic0600(s.file, b)
}

func (s *remoteDispatchStore) load(file string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.file = file
	b, err := os.ReadFile(file)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	var rows []persistedRemoteDispatch
	if err := json.Unmarshal(b, &rows); err != nil {
		return err
	}
	for _, row := range rows {
		if !bus.ValidDispatchID(row.ID) || s.m[row.ID] != nil {
			return fmt.Errorf("invalid remote dispatch journal identity")
		}
		if row.Lease != nil && (row.Lease.Owner == "" || (row.Lease.Handoff == nil && row.Lease.Worktree && row.Lease.Cwd != filepath.Join(configDir(), "dispatch-worktrees", row.ID))) {
			return fmt.Errorf("invalid remote dispatch journal destination")
		}
		s.m[row.ID] = &remoteDispatch{dispatchID: row.ID, sessionID: row.Session, seq: row.Seq, last: row.Last, lease: row.Lease, acknowledgedAt: row.AckedAt}
		if row.Session != "" {
			s.bySession[row.Session] = row.ID
		}
	}
	return nil
}

func (r *registry) dispatchPrepare(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Handoff      *handoffReceiptSelector `json:"handoff,omitempty"`
		RemoteOrigin *remoteOriginParam      `json:"remoteOrigin"`
		Cwd          string                  `json:"cwd"`
		Provider     string                  `json:"provider"`
		Worktree     bool                    `json:"worktree"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	id, err := r.acceptRemoteOrigin(spawnParams{RemoteOrigin: p.RemoteOrigin})
	if err != nil {
		return nil, err
	}
	if id == "" || p.RemoteOrigin.OwnerKey == "" {
		return nil, fmt.Errorf("authenticated paired origin required")
	}
	ready := false
	for _, provider := range r.dispatchProviderReadiness(ctx) {
		if provider.Provider == p.Provider && provider.Found && provider.Authenticated != nil && *provider.Authenticated {
			ready = true
		}
	}
	if !ready {
		return nil, fmt.Errorf("provider is not authenticated on this execution host")
	}
	if p.Handoff != nil {
		rec, err := r.preparedHandoff(p.RemoteOrigin.OwnerKey, id, p.Handoff)
		if err != nil {
			return nil, err
		}
		if p.Provider != rec.Plan.Provider || !p.Worktree {
			return nil, fmt.Errorf("handoff requires its bound provider and isolated worktree")
		}
		s := r.remote
		s.mu.Lock()
		defer s.mu.Unlock()
		if prior := s.m[id]; prior != nil {
			if prior.lease == nil || prior.lease.Handoff == nil || *prior.lease.Handoff != *p.Handoff || prior.lease.Owner != p.RemoteOrigin.OwnerKey || prior.lease.Claimed {
				return nil, fmt.Errorf("handoff admission already claimed or mismatched; do not repeat spawn")
			}
			return jsonResult(map[string]any{"cwd": prior.lease.Cwd, "repo": prior.lease.Repo, "worktree": true, "branch": prior.lease.Branch, "handoff": prior.lease.Handoff})
		}
		if len(s.m) >= 1024 {
			return nil, fmt.Errorf("dispatch journal capacity reached")
		}
		lease := &dispatchLease{Handoff: p.Handoff, Owner: p.RemoteOrigin.OwnerKey, Repo: rec.Allocation, Cwd: rec.Allocation, Provider: p.Provider, Worktree: true, Branch: "wks/handoff-" + id + "-input", Expires: time.Now().Add(5 * time.Minute).UnixMilli()}
		s.m[id] = &remoteDispatch{dispatchID: id, lease: lease}
		if err := s.persistLocked(); err != nil {
			delete(s.m, id)
			return nil, err
		}
		return jsonResult(map[string]any{"cwd": lease.Cwd, "repo": lease.Repo, "worktree": true, "branch": lease.Branch, "handoff": lease.Handoff})
	}
	known := false
	repositoryRoot := false
	for _, choice := range r.dispatchCwdChoices(ctx) {
		if choice.Path == p.Cwd && filepath.IsAbs(choice.Path) {
			known = true
			repositoryRoot = choice.Git
		}
	}
	if !known {
		return nil, fmt.Errorf("cwd must be an existing repository choice returned by this host")
	}
	root, err := filepath.EvalSymlinks(p.Cwd)
	if err != nil || root != filepath.Clean(p.Cwd) {
		return nil, fmt.Errorf("remote cwd must be canonical")
	}
	if p.Worktree {
		if !repositoryRoot {
			return nil, fmt.Errorf("remote isolated worktree allocation failed: select an actual repository root; no worker started")
		}
		actual, err := gitWorkRoot(ctx, root)
		if err != nil || filepath.Clean(actual) != root {
			return nil, fmt.Errorf("remote isolated worktree allocation failed: Git root differs from the selected directory; no worker started")
		}
	}
	s := r.remote
	s.mu.Lock()
	defer s.mu.Unlock()
	if prior := s.m[id]; prior != nil {
		if prior.lease == nil || prior.lease.Owner != p.RemoteOrigin.OwnerKey || prior.lease.Repo != root || prior.lease.Provider != p.Provider || prior.lease.Worktree != p.Worktree || prior.lease.Claimed {
			return nil, fmt.Errorf("dispatch already admitted or lease does not match; do not spawn again")
		}
		return jsonResult(map[string]any{"cwd": prior.lease.Cwd, "repo": prior.lease.Repo, "worktree": prior.lease.Worktree, "branch": prior.lease.Branch, "expires": prior.lease.Expires})
	}
	if len(s.m) >= 1024 {
		oldestID := ""
		var oldest int64
		for key, row := range s.m {
			if row.acknowledgedAt > 0 && (row.lease == nil || row.lease.Handoff == nil) && (oldestID == "" || row.acknowledgedAt < oldest) {
				oldestID, oldest = key, row.acknowledgedAt
			}
		}
		if oldestID != "" {
			delete(s.bySession, s.m[oldestID].sessionID)
			delete(s.m, oldestID)
		}
	}
	if len(s.m) >= 1024 {
		return nil, fmt.Errorf("remote dispatch journal is full; no worker was started")
	}
	lease := &dispatchLease{Owner: p.RemoteOrigin.OwnerKey, Repo: root, Cwd: root, Provider: p.Provider, Worktree: p.Worktree, Expires: time.Now().Add(5 * time.Minute).UnixMilli()}
	d := &remoteDispatch{dispatchID: id, lease: lease}
	s.m[id] = d
	if err := s.persistLocked(); err != nil {
		delete(s.m, id)
		return nil, err
	}
	if p.Worktree {
		lease.Cwd = filepath.Join(configDir(), "dispatch-worktrees", id)
		lease.Branch = "wks/paired-" + id
		if err := os.MkdirAll(filepath.Dir(lease.Cwd), 0700); err != nil {
			delete(s.m, id)
			_ = s.persistLocked()
			return nil, err
		}
		res, err := runGit(ctx, root, []string{"worktree", "add", "-b", lease.Branch, "--", lease.Cwd, "HEAD"})
		if err != nil || !res.ok {
			delete(s.m, id)
			_ = s.persistLocked()
			return nil, fmt.Errorf("remote isolated worktree allocation failed; no worker started")
		}
	}
	if err := s.persistLocked(); err != nil {
		if lease.Worktree {
			_, _ = runGit(ctx, root, []string{"worktree", "remove", "--force", "--", lease.Cwd})
		}
		delete(s.m, id)
		return nil, err
	}
	time.AfterFunc(5*time.Minute, func() { r.expireDispatchLease(id) })
	// No credential identity is returned to the origin or worker.
	return jsonResult(map[string]any{"cwd": lease.Cwd, "repo": lease.Repo, "worktree": lease.Worktree, "branch": lease.Branch, "expires": lease.Expires})
}

func (r *registry) expireDispatchLease(id string) {
	s := r.remote
	s.mu.Lock()
	defer s.mu.Unlock()
	d := s.m[id]
	if d == nil || d.lease == nil || d.lease.Handoff != nil || d.lease.Claimed || d.lease.Expires > time.Now().UnixMilli() {
		return
	}
	if d.lease.Worktree {
		if d.lease.Cwd != filepath.Join(configDir(), "dispatch-worktrees", id) {
			return
		}
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		defer cancel()
		res, err := runGit(ctx, d.lease.Repo, []string{"worktree", "remove", "--force", "--", d.lease.Cwd})
		if err != nil || !res.ok {
			time.AfterFunc(time.Minute, func() { r.expireDispatchLease(id) })
			return
		}
		if d.lease.Branch != "" {
			_, _ = runGit(ctx, d.lease.Repo, []string{"branch", "-d", "--", d.lease.Branch})
		}
	}
	delete(s.m, id)
	_ = s.persistLocked()
}

func (r *registry) claimDispatch(p spawnParams) error {
	if p.RemoteOrigin == nil {
		return nil
	}
	s := r.remote
	s.mu.Lock()
	defer s.mu.Unlock()
	d := s.m[p.RemoteOrigin.DispatchID]
	if d == nil || d.lease == nil || d.lease.Owner != p.RemoteOrigin.OwnerKey || d.lease.Cwd != p.Cwd || d.lease.Provider != p.Provider || d.lease.Claimed || d.lease.Expires <= time.Now().UnixMilli() {
		return fmt.Errorf("remote dispatch requires a matching unused lease; do not retry an uncertain spawn")
	}
	if (d.lease.Handoff == nil) != (p.Handoff == nil) || (p.Handoff != nil && *p.Handoff != *d.lease.Handoff) {
		return fmt.Errorf("spawn must consume the exact verified handoff receipt")
	}
	if p.Handoff != nil {
		if _, err := r.preparedHandoff(p.RemoteOrigin.OwnerKey, p.RemoteOrigin.DispatchID, p.Handoff); err != nil {
			return err
		}
	}
	d.lease.Claimed = true
	return s.persistLocked()
}

// Resume cleanup timers after a brain restart; unknown claimed admissions are
// retained for reconciliation and are never cleaned as abandoned worktrees.
func (r *registry) resumeDispatchLeases() {
	if r.remote == nil {
		return
	}
	r.remote.mu.Lock()
	defer r.remote.mu.Unlock()
	for id, d := range r.remote.m {
		if d.lease != nil && d.lease.Handoff != nil {
			binding, err := r.handoffBinding(d.lease.Handoff.Binding, d.lease.Owner)
			if err == nil {
				var rec handoffRecord
				b, readErr := os.ReadFile(filepath.Join(r.handoffDir(binding, id), "receipt.json"))
				if readErr == nil && json.Unmarshal(b, &rec) == nil && rec.AcceptedAt > 0 && !rec.Keep {
					r.scheduleHandoffCleanup(binding, id, rec.AcceptedAt)
				}
			}
			continue
		}
		if d.lease == nil || d.lease.Claimed {
			continue
		}
		id := id
		wait := time.Until(time.UnixMilli(d.lease.Expires))
		if wait < 0 {
			wait = 0
		}
		time.AfterFunc(wait, func() { r.expireDispatchLease(id) })
	}
}
