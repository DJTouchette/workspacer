package main

// The brain is the sole on-disk owner on both desktop (catalog scope) and
// execution hosts. The desktop orchestrates these bounded operations over the
// existing authenticated sockets; the model supplies a selection once.
import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/taskartifacts"
)

type handoffSelection struct {
	Name string `json:"name"`
	Kind string `json:"kind"`
}

type handoffReceiptSelector struct {
	Binding string `json:"binding"`
	Digest  string `json:"digest"`
}

func preparedHandoff(owner, task string, selector *handoffReceiptSelector) (*handoffRecord, error) {
	if selector == nil {
		return nil, fmt.Errorf("exact handoff receipt required")
	}
	binding, err := handoffBinding(selector.Binding, owner)
	if err != nil {
		return nil, err
	}
	b, err := os.ReadFile(filepath.Join(handoffDir(binding, task), "receipt.json"))
	if err != nil {
		return nil, fmt.Errorf("handoff preparation required")
	}
	var rec handoffRecord
	if taskartifacts.Decode(b, &rec) != nil || rec.Owner != owner || rec.State != "prepared" || rec.Plan.Input.Task != task || rec.Plan.Input.Origin != binding.Origin || rec.Plan.Revision != binding.Revision || rec.Digest != selector.Digest || rec.Allocation != filepath.Join(handoffDir(binding, task), "input-worktree") {
		return nil, fmt.Errorf("handoff receipt does not match this admission")
	}
	return &rec, nil
}

type handoffPlan struct {
	Version  int                    `json:"version"`
	Binding  string                 `json:"binding"`
	Revision string                 `json:"revision"`
	Provider string                 `json:"provider"`
	Input    taskartifacts.Manifest `json:"input"`
	Outputs  []handoffSelection     `json:"outputs"`
}

type handoffRecord struct {
	Owner      string                  `json:"owner"`
	Plan       handoffPlan             `json:"plan"`
	Digest     string                  `json:"digest"`
	State      string                  `json:"state"`
	Allocation string                  `json:"allocation,omitempty"`
	Result     *taskartifacts.Manifest `json:"result,omitempty"`
	Custody    string                  `json:"custody,omitempty"`
	AcceptedAt int64                   `json:"acceptedAt,omitempty"`
	Keep       bool                    `json:"keep,omitempty"`
}

type handoffRequest struct {
	Cwd        string                  `json:"cwd,omitempty"`
	Operation  string                  `json:"operation"`
	OriginKey  string                  `json:"originKey"`
	Binding    string                  `json:"binding"`
	Task       string                  `json:"task"`
	Plan       *handoffPlan            `json:"plan,omitempty"`
	Manifest   *taskartifacts.Manifest `json:"manifest,omitempty"`
	Provider   string                  `json:"provider,omitempty"`
	Selections []handoffSelection      `json:"selections,omitempty"`
	Outputs    []handoffSelection      `json:"outputs,omitempty"`
	Direction  string                  `json:"direction,omitempty"`
	Index      int                     `json:"index,omitempty"`
	Offset     int64                   `json:"offset,omitempty"`
	Data       []byte                  `json:"data,omitempty"`
	Digest     string                  `json:"digest,omitempty"`
	Keep       bool                    `json:"keep,omitempty"`
}

// Per-task locks avoid holding the dispatch journal mutex during I/O. The
// bounded journal admission below also bounds the number of retained locks.
var handoffLocks sync.Map

func handoffBinding(id, owner string) (taskartifacts.RepositoryBinding, error) {
	var bindings []taskartifacts.RepositoryBinding
	b, err := os.ReadFile(filepath.Join(configDir(), "handoff-bindings.json"))
	if err != nil || len(b) > 1<<20 || json.Unmarshal(b, &bindings) != nil {
		return taskartifacts.RepositoryBinding{}, fmt.Errorf("repository handoff setup required: install an approved handoff-bindings.json on both workspaces")
	}
	for _, binding := range bindings {
		if binding.ID == id && (binding.Owner == owner || owner == "local-host") {
			if err := binding.Validate(); err != nil {
				return binding, err
			}
			return binding, nil
		}
	}
	return taskartifacts.RepositoryBinding{}, fmt.Errorf("repository binding unavailable for this workspace owner")
}

func handoffDir(binding taskartifacts.RepositoryBinding, task string) string {
	return filepath.Join(configDir(), "task-handoffs", binding.Origin, task)
}

func saveHandoff(dir string, rec *handoffRecord) error {
	b, err := json.Marshal(rec)
	if err != nil {
		return err
	}
	return writeFileAtomic0600(filepath.Join(dir, "receipt.json"), b)
}

func planDigest(p handoffPlan) (string, error) {
	if p.Version != 1 || p.Binding == "" || p.Revision == "" || p.Provider == "" {
		return "", fmt.Errorf("unsupported handoff plan")
	}
	if err := p.Input.Validate(); err != nil {
		return "", err
	}
	if len(p.Outputs) > taskartifacts.MaxFiles {
		return "", fmt.Errorf("too many required outputs")
	}
	for _, out := range p.Outputs {
		if err := taskartifacts.ValidName(out.Name); err != nil {
			return "", err
		}
	}
	b, err := json.Marshal(p)
	return taskartifacts.Digest(b), err
}

func (r *registry) taskHandoff(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p handoffRequest
	if err := taskartifacts.Decode(raw, &p); err != nil {
		return nil, err
	}
	if !taskartifacts.ID.MatchString(p.Task) || p.OriginKey == "" {
		return nil, fmt.Errorf("authenticated task identity required")
	}
	binding, err := handoffBinding(p.Binding, p.OriginKey)
	if err != nil {
		return nil, err
	}
	dir := handoffDir(binding, p.Task)
	lock, _ := handoffLocks.LoadOrStore(dir, &sync.Mutex{})
	mu := lock.(*sync.Mutex)
	mu.Lock()
	defer mu.Unlock()
	var rec handoffRecord
	stored, readErr := os.ReadFile(filepath.Join(dir, "receipt.json"))
	if readErr == nil {
		if json.Unmarshal(stored, &rec) != nil || rec.Owner != p.OriginKey || rec.Plan.Input.Task != p.Task || rec.Plan.Binding != binding.ID || rec.Plan.Revision != binding.Revision || rec.Plan.Input.Origin != binding.Origin {
			return nil, fmt.Errorf("handoff receipt identity or binding revision mismatch")
		}
	} else if !os.IsNotExist(readErr) {
		return nil, readErr
	}
	if readErr != nil && p.Operation != "freeze" && p.Operation != "reserve" {
		return nil, fmt.Errorf("handoff unavailable for this owner")
	}
	switch p.Operation {
	case "freeze", "reserve":
		if readErr == nil {
			if p.Plan != nil {
				digest, err := planDigest(*p.Plan)
				if err != nil || digest != rec.Digest {
					return nil, fmt.Errorf("conflicting immutable handoff retry")
				}
			}
			return jsonResult(rec)
		}
		// Pending and sole-copy records never expire to make room. Operators
		// see capacity failure and can accept/clean completed work explicitly.
		entries, _ := os.ReadDir(filepath.Join(configDir(), "task-handoffs", binding.Origin))
		if len(entries) >= 32 {
			return nil, fmt.Errorf("handoff capacity reached: accept and clean retained work before admitting more")
		}
		if err := os.MkdirAll(dir, 0700); err != nil {
			return nil, err
		}
		if p.Operation == "freeze" {
			if p.OriginKey != "local-host" || !binding.Export {
				return nil, fmt.Errorf("local task export authority required")
			}
			if filepath.Clean(p.Cwd) != binding.Repository {
				return nil, fmt.Errorf("selected source workspace does not match the approved repository binding")
			}
			commit, format, err := taskartifacts.CheckSource(ctx, binding.Repository)
			if err != nil {
				return nil, err
			}
			if err := taskartifacts.VerifyTree(ctx, binding.Repository, commit); err != nil {
				return nil, err
			}
			m, err := freezeHandoffArtifacts(binding, p.Task, commit, format, p.Selections, filepath.Join(binding.Repository, ".workspacer", "reports"), filepath.Join(dir, "input"))
			if err != nil {
				return nil, err
			}
			rec = handoffRecord{Owner: p.OriginKey, Plan: handoffPlan{1, binding.ID, binding.Revision, p.Provider, m, p.Outputs}, State: "frozen"}
			rec.Digest, err = planDigest(rec.Plan)
			if err != nil {
				return nil, err
			}
			// Retain the immutable plan before publication. Lost push replies
			// resume the same generated ref; no alternative remote is selected.
			if err := saveHandoff(dir, &rec); err != nil {
				return nil, err
			}
		} else {
			if !binding.Import || p.Plan == nil || p.Plan.Input.Task != p.Task || p.Plan.Input.Origin != binding.Origin || p.Plan.Binding != binding.ID || p.Plan.Revision != binding.Revision {
				return nil, fmt.Errorf("handoff import mapping mismatch")
			}
			digest, err := planDigest(*p.Plan)
			if err != nil {
				return nil, err
			}
			rec = handoffRecord{Owner: p.OriginKey, Plan: *p.Plan, Digest: digest, State: "transferring"}
			if err := os.Mkdir(filepath.Join(dir, "input"), 0700); err != nil && !os.IsExist(err) {
				return nil, err
			}
			if err := saveHandoff(dir, &rec); err != nil {
				return nil, err
			}
		}
		return jsonResult(rec)
	case "publish":
		if !binding.Export || p.OriginKey != "local-host" {
			return nil, fmt.Errorf("source publication requires local export authority")
		}
		ref, _ := binding.Ref(p.Task, "input")
		_, err := taskartifacts.Git(ctx, binding.Repository, binding.CredentialHelper, "push", "--porcelain", "--force-with-lease="+ref+":", "--", binding.Remote, rec.Plan.Input.Commit+":"+ref)
		if err != nil {
			// A lost successful push is accepted only at the exact expected ID.
			got, e := taskartifacts.Git(ctx, binding.Repository, binding.CredentialHelper, "ls-remote", "--refs", "--", binding.Remote, ref)
			fields := strings.Fields(string(got))
			if e != nil || len(fields) != 2 || fields[0] != rec.Plan.Input.Commit || fields[1] != ref {
				return nil, err
			}
		}
		return jsonResult(rec)
	case "write", "read":
		m := rec.Plan.Input
		if p.Direction == "result" {
			if rec.Result == nil {
				return nil, fmt.Errorf("result not sealed")
			}
			m = *rec.Result
		} else if p.Direction != "input" {
			return nil, fmt.Errorf("invalid transfer direction")
		}
		store, err := taskartifacts.Open(filepath.Join(dir, p.Direction), m)
		if err != nil {
			return nil, err
		}
		defer store.Close()
		if p.Operation == "write" {
			if (p.Direction == "input" && rec.State != "transferring") || (p.Direction == "result" && rec.State != "receiving-result") {
				return nil, fmt.Errorf("sealed task bytes cannot be changed")
			}
			if err := store.Write(p.Index, p.Offset, p.Data); err != nil {
				return nil, err
			}
			return jsonResult(map[string]any{"offset": p.Offset + int64(len(p.Data))})
		}
		if err := store.Verify(); err != nil {
			return nil, err
		}
		b, err := store.Read(p.Index, p.Offset)
		if err != nil {
			return nil, err
		}
		return jsonResult(map[string]any{"data": b})
	case "prepare":
		if rec.State == "prepared" {
			return jsonResult(rec)
		}
		if rec.State != "transferring" || !binding.Import {
			return nil, fmt.Errorf("handoff cannot prepare in this state")
		}
		store, err := taskartifacts.Open(filepath.Join(dir, "input"), rec.Plan.Input)
		if err != nil {
			return nil, err
		}
		defer store.Close()
		if err := store.Verify(); err != nil {
			return nil, err
		}
		allocation, err := importHandoffCode(ctx, binding, dir, p.Task, "input", rec.Plan.Input)
		if err != nil {
			return nil, err
		}
		if err := store.Materialize(filepath.Join(allocation, ".workspacer", "handoffs", p.Task)); err != nil {
			return nil, err
		}
		rec.Allocation, rec.State = allocation, "prepared"
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	case "status":
		return jsonResult(rec)
	case "sealResult":
		if !binding.Export || rec.Allocation == "" {
			return nil, fmt.Errorf("result export is not authorized")
		}
		if rec.State == "result-sealed" {
			return jsonResult(rec)
		}
		if r.remote == nil {
			return nil, fmt.Errorf("execution journal unavailable")
		}
		r.remote.mu.Lock()
		dispatch := r.remote.m[p.Task]
		finished := dispatch != nil && dispatch.lease != nil && dispatch.lease.Owner == p.OriginKey && dispatch.lease.Claimed && dispatch.last != nil && dispatch.last.Final
		r.remote.mu.Unlock()
		if !finished {
			return nil, fmt.Errorf("worker outcome unresolved; result retained without sealing")
		}
		commit, format, err := taskartifacts.CheckSource(ctx, rec.Allocation)
		if err != nil {
			rec.State = "needs-checkpoint"
			if e := saveHandoff(dir, &rec); e != nil {
				return nil, e
			}
			return jsonResult(rec)
		}
		if _, err := taskartifacts.Git(ctx, rec.Allocation, "", "merge-base", "--is-ancestor", rec.Plan.Input.Commit, commit); err != nil {
			return nil, fmt.Errorf("result is not a descendant of the selected input checkpoint")
		}
		if err := taskartifacts.VerifyTree(ctx, rec.Allocation, commit); err != nil {
			return nil, err
		}
		manifest, err := freezeHandoffArtifacts(binding, p.Task, commit, format, rec.Plan.Outputs, filepath.Join(rec.Allocation, ".workspacer", "handoffs", p.Task), filepath.Join(dir, "result"))
		if err != nil {
			return nil, err
		}
		rec.Result = &manifest
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		ref, _ := binding.Ref(p.Task, "result")
		if err := publishHandoffRef(ctx, binding, rec.Allocation, commit, ref); err != nil {
			return nil, err
		}
		rec.State = "result-sealed"
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	case "receiveResult":
		if p.OriginKey != "local-host" || !binding.Import || p.Manifest == nil {
			return nil, fmt.Errorf("local result import authority required")
		}
		m := *p.Manifest
		if err := m.Validate(); err != nil {
			return nil, err
		}
		if m.Task != p.Task || m.Origin != rec.Plan.Input.Origin || m.ObjectFormat != rec.Plan.Input.ObjectFormat {
			return nil, fmt.Errorf("result ownership or Git object format mismatch")
		}
		if len(m.Entries) != len(rec.Plan.Outputs) {
			return nil, fmt.Errorf("required result artifacts missing")
		}
		for i, out := range rec.Plan.Outputs {
			if m.Entries[i].Name != out.Name || m.Entries[i].Kind != out.Kind {
				return nil, fmt.Errorf("result artifact selection mismatch")
			}
		}
		if rec.Result != nil {
			a, _ := rec.Result.Seal()
			b, _ := m.Seal()
			if a != b {
				return nil, fmt.Errorf("conflicting sealed result")
			}
			return jsonResult(rec)
		}
		rec.Result, rec.State = &m, "receiving-result"
		if err := os.MkdirAll(filepath.Join(dir, "result"), 0700); err != nil {
			return nil, err
		}
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	case "importResult":
		if p.OriginKey != "local-host" || !binding.Import || rec.Result == nil {
			return nil, fmt.Errorf("local result import authority required")
		}
		if rec.State == "received" {
			return jsonResult(rec)
		}
		store, err := taskartifacts.Open(filepath.Join(dir, "result"), *rec.Result)
		if err != nil {
			return nil, err
		}
		defer store.Close()
		if err := store.Verify(); err != nil {
			return nil, err
		}
		allocation, err := importHandoffCode(ctx, binding, dir, p.Task, "result", *rec.Result)
		if err != nil {
			return nil, err
		}
		if _, err := taskartifacts.Git(ctx, allocation, "", "merge-base", "--is-ancestor", rec.Plan.Input.Commit, rec.Result.Commit); err != nil {
			return nil, fmt.Errorf("returned checkpoint does not descend from selected source")
		}
		if err := store.Materialize(filepath.Join(allocation, ".workspacer", "handoffs", p.Task)); err != nil {
			return nil, err
		}
		rec.Allocation, rec.State = allocation, "received"
		rec.Custody, _ = rec.Result.Seal()
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	case "custody":
		if rec.Result == nil || rec.State != "result-sealed" {
			return nil, fmt.Errorf("result custody cannot acknowledge unsealed outputs")
		}
		digest, _ := rec.Result.Seal()
		if p.Digest != digest {
			return nil, fmt.Errorf("custody must acknowledge the exact result manifest")
		}
		rec.Custody = digest
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	case "disposition":
		if rec.Result == nil || rec.Custody == "" || p.Digest != rec.Custody {
			return nil, fmt.Errorf("accepted disposition requires durable custody of this exact result")
		}
		rec.Keep = p.Keep
		if !p.Keep && rec.AcceptedAt == 0 {
			rec.AcceptedAt = time.Now().UnixMilli()
		}
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	default:
		return nil, fmt.Errorf("unsupported handoff operation")
	}
}

func publishHandoffRef(ctx context.Context, binding taskartifacts.RepositoryBinding, repo, commit, ref string) error {
	_, err := taskartifacts.Git(ctx, repo, binding.CredentialHelper, "push", "--porcelain", "--force-with-lease="+ref+":", "--", binding.Remote, commit+":"+ref)
	if err == nil {
		return nil
	}
	got, checkErr := taskartifacts.Git(ctx, repo, binding.CredentialHelper, "ls-remote", "--refs", "--", binding.Remote, ref)
	fields := strings.Fields(string(got))
	if checkErr != nil || len(fields) != 2 || fields[0] != commit || fields[1] != ref {
		return err
	}
	return nil
}

func freezeHandoffArtifacts(binding taskartifacts.RepositoryBinding, task, commit, format string, selections []handoffSelection, source, dest string) (taskartifacts.Manifest, error) {
	m := taskartifacts.Manifest{Version: 1, Task: task, Origin: binding.Origin, Producer: task, Commit: commit, ObjectFormat: format, Entries: []taskartifacts.Entry{}}
	if len(selections) > taskartifacts.MaxFiles {
		return m, fmt.Errorf("too many selected artifacts")
	}
	if err := os.MkdirAll(dest, 0700); err != nil {
		return m, err
	}
	if len(selections) == 0 {
		return m, nil
	}
	root, err := os.OpenRoot(source)
	if err != nil {
		return m, err
	}
	defer root.Close()
	var total int
	for i, selection := range selections {
		b, err := taskartifacts.ReadSelected(root, selection.Name)
		if err != nil {
			return m, err
		}
		total += len(b)
		if total > taskartifacts.TaskBytes {
			return m, fmt.Errorf("selected task exceeds artifact capacity")
		}
		m.Entries = append(m.Entries, taskartifacts.Entry{Name: selection.Name, Kind: selection.Kind, Size: int64(len(b)), SHA256: taskartifacts.Digest(b)})
		if err := m.Validate(); err != nil {
			return m, err
		}
		if err := writeFileAtomic0600(filepath.Join(dest, fmt.Sprintf("%d.bytes", i)), b); err != nil {
			return m, err
		}
	}
	return m, nil
}

func importHandoffCode(ctx context.Context, binding taskartifacts.RepositoryBinding, dir, task, direction string, m taskartifacts.Manifest) (string, error) {
	// A fresh private Git repository quarantines remote objects/config. It is
	// never the receiver's active checkout or the source's index/worktree.
	repo := filepath.Join(dir, direction+"-git")
	allocation := filepath.Join(dir, direction+"-worktree")
	if err := os.MkdirAll(repo, 0700); err != nil {
		return "", err
	}
	if _, err := taskartifacts.Git(ctx, repo, "", "init", "--bare", "--template=", "--object-format="+m.ObjectFormat); err != nil {
		return "", err
	}
	ref, _ := binding.Ref(task, direction)
	if _, err := taskartifacts.Git(ctx, repo, binding.CredentialHelper, "fetch", "--no-tags", "--no-recurse-submodules", "--", binding.Remote, ref+":refs/handoff/verified"); err != nil {
		return "", err
	}
	got, err := taskartifacts.Git(ctx, repo, "", "rev-parse", "--verify", "refs/handoff/verified^{commit}")
	if err != nil || strings.TrimSpace(string(got)) != m.Commit {
		return "", fmt.Errorf("handoff ref moved or exact commit missing; no worker started")
	}
	if err := taskartifacts.VerifyTree(ctx, repo, m.Commit); err != nil {
		return "", err
	}
	if err := os.WriteFile(filepath.Join(repo, "info", "exclude"), []byte("/.workspacer/handoffs/\n"), 0600); err != nil {
		return "", err
	}
	if _, err := taskartifacts.Git(ctx, repo, "", "worktree", "add", "-b", "wks/handoff-"+task+"-"+direction, "--", allocation, m.Commit); err != nil {
		return "", err
	}
	status, err := taskartifacts.Git(ctx, allocation, "", "status", "--porcelain=v1", "--untracked-files=all")
	if err != nil || len(status) != 0 {
		return "", fmt.Errorf("materialized checkpoint verification failed")
	}
	return allocation, nil
}

// Seven days starts at accepted disposition, never terminal-message ACK.
const handoffCleanupGrace = 7 * 24 * time.Hour
