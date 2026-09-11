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
	Version      int    `json:"version"`
	AllocationId string `json:"allocationId"`
	Binding      string `json:"binding"`
	Digest       string `json:"digest"`
}

func (r *registry) preparedHandoff(ctx context.Context, owner, task string, selector *handoffReceiptSelector) (*handoffRecord, error) {
	if selector == nil || selector.Version != 1 {
		return nil, fmt.Errorf("exact handoff receipt required")
	}
	binding, err := r.handoffBinding(selector.Binding, owner)
	if err != nil {
		return nil, err
	}
	b, err := os.ReadFile(filepath.Join(r.handoffDir(binding, task), "receipt.json"))
	if err != nil {
		return nil, fmt.Errorf("handoff preparation required")
	}
	var rec handoffRecord
	if taskartifacts.Decode(b, &rec) != nil || rec.Owner != owner || rec.State != "prepared" || rec.Plan.Input.Task != task || rec.Plan.Input.Origin != binding.Origin || rec.Plan.Revision != binding.Revision || rec.Digest != selector.Digest || rec.Allocation != filepath.Join(r.handoffDir(binding, task), "input-worktree") {
		return nil, fmt.Errorf("handoff receipt does not match this admission")
	}
	identity, err := taskartifacts.DirectoryIdentity(rec.Allocation)
	if err != nil || identity != rec.AllocationId || identity != selector.AllocationId {
		return nil, fmt.Errorf("prepared allocation identity changed")
	}
	if err := verifyHandoffAllocation(ctx, rec.Allocation, filepath.Join(r.handoffDir(binding, task), "input-git"), task, "input", rec.Plan.Input); err != nil {
		return nil, err
	}
	if err := verifyHandoffEvidence(rec.Allocation, task, rec.Plan.Input); err != nil {
		return nil, err
	}
	return &rec, nil
}

func verifyHandoffEvidence(allocation, task string, m taskartifacts.Manifest) error {
	if err := taskartifacts.VerifyPrivateTree(allocation); err != nil {
		return err
	}
	root, err := taskartifacts.OpenSelectedRoot(allocation, ".workspacer/handoffs/"+task)
	if err != nil {
		return err
	}
	defer root.Close()
	for _, e := range m.Entries {
		b, err := taskartifacts.ReadSelected(root, e.Name)
		if err != nil || int64(len(b)) != e.Size || taskartifacts.Digest(b) != e.SHA256 {
			return fmt.Errorf("prepared artifact changed: %s", e.Name)
		}
	}
	return nil
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
	SourceRepo   string                       `json:"sourceRepo,omitempty"`
	InputPin     string                       `json:"inputPin,omitempty"`
	Storage      *taskartifacts.StorageStatus `json:"storage,omitempty"`
	AllocationId string                       `json:"allocationId,omitempty"`
	Predecessor  string                       `json:"predecessor,omitempty"`
	Owner        string                       `json:"owner"`
	Plan         handoffPlan                  `json:"plan"`
	Digest       string                       `json:"digest"`
	State        string                       `json:"state"`
	Allocation   string                       `json:"allocation,omitempty"`
	Result       *taskartifacts.Manifest      `json:"result,omitempty"`
	Custody      string                       `json:"custody,omitempty"`
	AcceptedAt   int64                        `json:"acceptedAt,omitempty"`
	Keep         bool                         `json:"keep,omitempty"`
	Cleaning     bool                         `json:"cleaning,omitempty"`
}

type handoffRequest struct {
	FromTask   string                  `json:"fromTask,omitempty"`
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
var handoffLocks [64]sync.Mutex

func (r *registry) handoffBinding(id, owner string) (taskartifacts.RepositoryBinding, error) {
	var bindings []taskartifacts.RepositoryBinding
	b, err := os.ReadFile(filepath.Join(r.handoffConfigDir(), "handoff-bindings.json"))
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

func (r *registry) handoffDir(binding taskartifacts.RepositoryBinding, task string) string {
	return filepath.Join(r.handoffConfigDir(), "task-handoffs", binding.Origin, task)
}

func saveHandoff(dir string, rec *handoffRecord) error {
	b, err := json.Marshal(rec)
	if err != nil {
		return err
	}
	return writeHandoffAtomic(filepath.Join(dir, "receipt.json"), b)
}

// Receipt success means durable bytes, including the containing directory
// entry. This is deliberately stricter than UI preference persistence.
func writeHandoffAtomic(name string, b []byte) error {
	dir := filepath.Dir(name)
	f, err := os.CreateTemp(dir, ".handoff-")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer os.Remove(tmp)
	if _, err = f.Write(b); err == nil {
		err = f.Sync()
	}
	closeErr := f.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	return taskartifacts.CommitFile(tmp, name)
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
	outputs := p.Input
	outputs.Entries = nil
	for _, out := range p.Outputs {
		outputs.Entries = append(outputs.Entries, taskartifacts.Entry{Name: out.Name, Kind: out.Kind, SHA256: taskartifacts.Digest(nil)})
	}
	if err := outputs.Validate(); err != nil {
		return "", err
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
	binding, err := r.handoffBinding(p.Binding, p.OriginKey)
	if err != nil {
		return nil, err
	}
	if err := taskartifacts.MakePrivateDirectory(filepath.Join(r.handoffConfigDir(), "task-handoffs")); err != nil {
		return nil, err
	}
	dir := r.handoffDir(binding, p.Task)
	key := taskartifacts.Digest([]byte(dir))
	mu := &handoffLocks[int(key[0])%len(handoffLocks)]
	mu.Lock()
	defer mu.Unlock()
	var rec handoffRecord
	stored, readErr := os.ReadFile(filepath.Join(dir, "receipt.json"))
	if readErr == nil {
		if err := taskartifacts.VerifyPrivateDirectory(dir); err != nil {
			return nil, err
		}
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
			if p.Operation == "freeze" {
				if p.OriginKey != "local-host" || !binding.Export || filepath.Clean(p.Cwd) != binding.Repository || p.FromTask != rec.Predecessor || p.Provider != rec.Plan.Provider || len(p.Selections) != len(rec.Plan.Input.Entries) || len(p.Outputs) != len(rec.Plan.Outputs) {
					return nil, fmt.Errorf("conflicting immutable source retry")
				}
				for i, selection := range p.Selections {
					entry := rec.Plan.Input.Entries[i]
					if selection.Name != entry.Name || selection.Kind != entry.Kind {
						return nil, fmt.Errorf("conflicting input selection retry")
					}
				}
				for i, output := range p.Outputs {
					if output != rec.Plan.Outputs[i] {
						return nil, fmt.Errorf("conflicting required output retry")
					}
				}
				if rec.State == "pinning" {
					if err := finishHandoffPin(ctx, &rec, dir); err != nil {
						return nil, err
					}
				}
			}
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
		entries, _ := os.ReadDir(filepath.Join(r.handoffConfigDir(), "task-handoffs", binding.Origin))
		if len(entries) >= 32 {
			return nil, fmt.Errorf("handoff capacity reached: accept and clean retained work before admitting more")
		}
		if err := taskartifacts.MakePrivateDirectory(dir); err != nil {
			return nil, err
		}
		release, err := taskartifacts.ReserveStorage(filepath.Dir(filepath.Dir(dir)), dir)
		if err != nil {
			return nil, err
		}
		defer release()
		if p.Operation == "freeze" {
			if p.OriginKey != "local-host" || !binding.Export {
				return nil, fmt.Errorf("local task export authority required")
			}
			if filepath.Clean(p.Cwd) != binding.Repository {
				return nil, fmt.Errorf("selected source workspace does not match the approved repository binding")
			}
			source, folder := binding.Repository, ".workspacer/reports"
			expectedCommit := ""
			expectedArtifacts := map[string]string{}
			if p.FromTask != "" {
				if !taskartifacts.ID.MatchString(p.FromTask) {
					return nil, fmt.Errorf("invalid predecessor task")
				}
				b, err := os.ReadFile(filepath.Join(r.handoffDir(binding, p.FromTask), "receipt.json"))
				var previous handoffRecord
				if err != nil || taskartifacts.Decode(b, &previous) != nil || previous.Owner != "local-host" || previous.Plan.Binding != binding.ID || previous.State != "received" || previous.Custody == "" || previous.Result == nil {
					return nil, fmt.Errorf("predecessor output is not in verified local custody")
				}
				if err := verifyRecordedAllocation(&previous); err != nil {
					return nil, err
				}
				source, folder = previous.Allocation, ".workspacer/handoffs/"+p.FromTask
				expectedCommit = previous.Result.Commit
				for _, entry := range previous.Result.Entries {
					expectedArtifacts[entry.Name] = entry.SHA256
				}
				for _, selection := range p.Selections {
					selected := false
					for _, entry := range previous.Result.Entries {
						if entry.Name == selection.Name && entry.Kind == selection.Kind {
							selected = true
						}
					}
					if !selected {
						return nil, fmt.Errorf("artifact was not returned by the selected predecessor")
					}
				}
			}
			selectedPaths := make([]string, 0, len(p.Selections))
			for _, selection := range p.Selections {
				if err := taskartifacts.ValidName(selection.Name); err != nil {
					return nil, err
				}
				selectedPaths = append(selectedPaths, folder+"/"+selection.Name)
			}
			commit, format, err := taskartifacts.CheckSource(ctx, source, selectedPaths...)
			if err != nil {
				return nil, err
			}
			if expectedCommit != "" && commit != expectedCommit {
				return nil, fmt.Errorf("predecessor checkpoint changed since custody verification")
			}
			if err := taskartifacts.VerifyTree(ctx, source, commit); err != nil {
				return nil, err
			}
			m, err := freezeHandoffArtifacts(binding, p.Task, commit, format, p.Selections, source, folder, filepath.Join(dir, "input"))
			if err != nil {
				return nil, err
			}
			if p.FromTask != "" {
				m.Producer = p.FromTask
				for _, entry := range m.Entries {
					if expectedArtifacts[entry.Name] != entry.SHA256 {
						return nil, fmt.Errorf("predecessor artifact changed since custody verification")
					}
				}
			}
			after, afterFormat, err := taskartifacts.CheckSource(ctx, source, selectedPaths...)
			if err != nil || after != commit || afterFormat != format {
				return nil, fmt.Errorf("source checkpoint changed during freeze")
			}
			rec = handoffRecord{Owner: p.OriginKey, Plan: handoffPlan{1, binding.ID, binding.Revision, p.Provider, m, p.Outputs}, State: "frozen"}
			rec.Predecessor = p.FromTask
			common, err := taskartifacts.Git(ctx, source, "", "rev-parse", "--path-format=absolute", "--git-common-dir")
			if err != nil {
				return nil, err
			}
			rec.SourceRepo = filepath.Clean(strings.TrimSpace(string(common)))
			rec.InputPin, err = binding.Ref(p.Task, "input")
			if err != nil {
				return nil, err
			}
			rec.Digest, err = planDigest(rec.Plan)
			if err != nil {
				return nil, err
			}
			// Retain the immutable plan before publication. Lost push replies
			// resume the same generated ref; no alternative remote is selected.
			rec.State = "pinning"
			if err := saveHandoff(dir, &rec); err != nil {
				return nil, err
			}
			if err := finishHandoffPin(ctx, &rec, dir); err != nil {
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
		source := binding.Repository
		if rec.SourceRepo != "" {
			source = rec.SourceRepo
		}
		if rec.Predecessor != "" {
			source = filepath.Join(r.handoffDir(binding, rec.Predecessor), "result-git")
		}
		ref, _ := binding.Ref(p.Task, "input")
		_, err := binding.GitRemote(ctx, source, "push", "--porcelain", "--force-with-lease="+ref+":", "--", binding.Remote, rec.Plan.Input.Commit+":"+ref)
		if err != nil {
			// A lost successful push is accepted only at the exact expected ID.
			got, e := binding.GitRemote(ctx, source, "ls-remote", "--refs", "--", binding.Remote, ref)
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
			release, err := taskartifacts.ReserveStorage(filepath.Dir(filepath.Dir(dir)), dir)
			if err != nil {
				return nil, err
			}
			defer release()
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
	case "prepare", "prepareLocal":
		if rec.State == "prepared" {
			return jsonResult(rec)
		}
		local := p.Operation == "prepareLocal" && p.OriginKey == "local-host" && rec.Predecessor != ""
		if (!local && rec.State != "transferring") || (local && rec.State != "frozen") || !binding.Import {
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
		localSource := ""
		if local {
			localSource = filepath.Join(r.handoffDir(binding, rec.Predecessor), "result-git")
		}
		allocation, err := importHandoffCode(ctx, binding, dir, p.Task, "input", rec.Plan.Input, localSource)
		if err != nil {
			return nil, err
		}
		if err := store.Materialize(filepath.Join(allocation, ".workspacer", "handoffs", p.Task)); err != nil {
			return nil, err
		}
		if err := taskartifacts.VerifyPrivateTree(allocation); err != nil {
			return nil, err
		}
		rec.Allocation, rec.State = allocation, "prepared"
		rec.AllocationId, err = taskartifacts.DirectoryIdentity(allocation)
		if err != nil {
			return nil, err
		}
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	case "claimLocal":
		if err := verifyRecordedAllocation(&rec); err != nil {
			return nil, err
		}
		if p.OriginKey != "local-host" || rec.Predecessor == "" || rec.State != "prepared" || rec.Digest != p.Digest {
			return nil, fmt.Errorf("local admission already claimed or receipt mismatched; do not repeat an uncertain spawn")
		}
		if err := verifyHandoffAllocation(ctx, rec.Allocation, filepath.Join(dir, "input-git"), p.Task, "input", rec.Plan.Input); err != nil {
			return nil, err
		}
		if err := verifyHandoffEvidence(rec.Allocation, p.Task, rec.Plan.Input); err != nil {
			return nil, err
		}
		rec.State = "admitted"
		if err := saveHandoff(dir, &rec); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	case "status":
		status, err := taskartifacts.InspectStorage(filepath.Dir(filepath.Dir(dir)))
		if err != nil {
			return nil, err
		}
		rec.Storage = &status
		return jsonResult(rec)
	case "sealResult":
		if !binding.Export || rec.Allocation == "" {
			return nil, fmt.Errorf("result export is not authorized")
		}
		if rec.State != "prepared" && rec.State != "needs-checkpoint" && rec.State != "result-sealed" {
			return nil, fmt.Errorf("handoff is not an execution result eligible for sealing")
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
		// The manifest and spool were frozen before publication. A crash or
		// lost push reply must resume that exact result, even if the worker's
		// artifact files subsequently change. Never refreeze a pending result.
		if rec.Result != nil {
			ref, _ := binding.Ref(p.Task, "result")
			if err := publishHandoffRef(ctx, binding, rec.Allocation, rec.Result.Commit, ref); err != nil {
				return nil, err
			}
			rec.State = "result-sealed"
			if err := saveHandoff(dir, &rec); err != nil {
				return nil, err
			}
			return jsonResult(rec)
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
		manifest, err := freezeHandoffArtifacts(binding, p.Task, commit, format, rec.Plan.Outputs, rec.Allocation, ".workspacer/handoffs/"+p.Task, filepath.Join(dir, "result"))
		if err != nil {
			return nil, err
		}
		after, afterFormat, err := taskartifacts.CheckSource(ctx, rec.Allocation)
		if err != nil || after != commit || afterFormat != format {
			return nil, fmt.Errorf("result checkpoint changed during sealing")
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
			if err := verifyRecordedAllocation(&rec); err != nil {
				return nil, err
			}
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
		allocation, err := importHandoffCode(ctx, binding, dir, p.Task, "result", *rec.Result, "")
		if err != nil {
			return nil, err
		}
		if _, err := taskartifacts.Git(ctx, allocation, "", "merge-base", "--is-ancestor", rec.Plan.Input.Commit, rec.Result.Commit); err != nil {
			return nil, fmt.Errorf("returned checkpoint does not descend from selected source")
		}
		if err := store.Materialize(filepath.Join(allocation, ".workspacer", "handoffs", p.Task)); err != nil {
			return nil, err
		}
		if err := taskartifacts.VerifyPrivateTree(allocation); err != nil {
			return nil, err
		}
		rec.Allocation, rec.State = allocation, "received"
		rec.AllocationId, err = taskartifacts.DirectoryIdentity(allocation)
		if err != nil {
			return nil, err
		}
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
		if !p.Keep {
			r.scheduleHandoffCleanup(binding, p.Task, rec.AcceptedAt)
		}
		return jsonResult(rec)
	case "cleanup":
		if rec.State == "cleaned" {
			return jsonResult(rec)
		}
		if err := r.cleanupHandoff(ctx, binding, &rec, time.Now()); err != nil {
			return nil, err
		}
		return jsonResult(rec)
	default:
		return nil, fmt.Errorf("unsupported handoff operation")
	}
}

func publishHandoffRef(ctx context.Context, binding taskartifacts.RepositoryBinding, repo, commit, ref string) error {
	_, err := binding.GitRemote(ctx, repo, "push", "--porcelain", "--force-with-lease="+ref+":", "--", binding.Remote, commit+":"+ref)
	if err == nil {
		return nil
	}
	got, checkErr := binding.GitRemote(ctx, repo, "ls-remote", "--refs", "--", binding.Remote, ref)
	fields := strings.Fields(string(got))
	if checkErr != nil || len(fields) != 2 || fields[0] != commit || fields[1] != ref {
		return err
	}
	return nil
}

func deleteHandoffRef(ctx context.Context, binding taskartifacts.RepositoryBinding, repo, commit, ref string) error {
	got, err := binding.GitRemote(ctx, repo, "ls-remote", "--refs", "--", binding.Remote, ref)
	if err != nil {
		return err
	}
	fields := strings.Fields(string(got))
	if len(fields) == 0 {
		return nil
	} // A previous compare-delete succeeded.
	if len(fields) != 2 || fields[0] != commit || fields[1] != ref {
		return fmt.Errorf("cleanup blocked: generated ref changed")
	}
	_, err = binding.GitRemote(ctx, repo, "push", "--porcelain", "--force-with-lease="+ref+":"+commit, "--", binding.Remote, ":"+ref)
	return err
}

func freezeHandoffArtifacts(binding taskartifacts.RepositoryBinding, task, commit, format string, selections []handoffSelection, sourceBase, sourceRelative, dest string) (taskartifacts.Manifest, error) {
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
	root, err := taskartifacts.OpenSelectedRoot(sourceBase, sourceRelative)
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
		if err := writeHandoffAtomic(filepath.Join(dest, fmt.Sprintf("%d.bytes", i)), b); err != nil {
			return m, err
		}
	}
	return m, nil
}

func importHandoffCode(ctx context.Context, binding taskartifacts.RepositoryBinding, dir, task, direction string, m taskartifacts.Manifest, localSource string) (string, error) {
	release, err := taskartifacts.ReserveStorage(filepath.Dir(filepath.Dir(dir)), dir)
	if err != nil {
		return "", err
	}
	defer release()
	ctx = taskartifacts.WithStorageLimit(ctx, dir)
	// A fresh private Git repository quarantines remote objects/config. It is
	// never the receiver's active checkout or the source's index/worktree.
	repo := filepath.Join(dir, direction+"-git")
	allocation := filepath.Join(dir, direction+"-worktree")
	if _, err := os.Lstat(allocation); err == nil {
		if err := verifyHandoffAllocation(ctx, allocation, repo, task, direction, m); err != nil {
			return "", err
		}
		return allocation, nil
	} else if !os.IsNotExist(err) {
		return "", err
	}
	if err := os.MkdirAll(repo, 0700); err != nil {
		return "", err
	}
	if err := taskartifacts.VerifyPrivateDirectory(repo); err != nil {
		return "", err
	}
	if _, err := taskartifacts.Git(ctx, repo, "", "init", "--bare", "--template=", "--object-format="+m.ObjectFormat); err != nil {
		return "", err
	}
	ref, _ := binding.Ref(task, direction)
	if localSource != "" {
		// This path is derived only from an origin-owned verified custody
		// receipt. It is never a peer URL or a caller filesystem grant.
		if _, err := taskartifacts.Git(ctx, repo, "", "-c", "protocol.file.allow=always", "fetch", "--no-tags", "--no-recurse-submodules", "--", localSource, m.Commit+":refs/handoff/verified"); err != nil {
			return "", err
		}
	} else {
		if _, err := binding.GitRemote(ctx, repo, "fetch", "--no-tags", "--no-recurse-submodules", "--", binding.Remote, ref+":refs/handoff/verified"); err != nil {
			return "", err
		}
	}
	got, err := taskartifacts.Git(ctx, repo, "", "rev-parse", "--verify", "refs/handoff/verified^{commit}")
	if err != nil || strings.TrimSpace(string(got)) != m.Commit {
		return "", fmt.Errorf("handoff ref moved or exact commit missing; no worker started")
	}
	if err := taskartifacts.VerifyTree(ctx, repo, m.Commit); err != nil {
		return "", err
	}
	if err := os.MkdirAll(filepath.Join(repo, "info"), 0700); err != nil {
		return "", err
	}
	if err := os.WriteFile(filepath.Join(repo, "info", "exclude"), []byte("/.workspacer/handoffs/\n"), 0600); err != nil {
		return "", err
	}
	if _, err := taskartifacts.Git(ctx, repo, "", "worktree", "add", "-b", "wks/handoff-"+task+"-"+direction, "--", allocation, m.Commit); err != nil {
		return "", err
	}
	if err := os.Chmod(allocation, 0700); err != nil {
		return "", err
	}
	if err := taskartifacts.VerifyPrivateTree(allocation); err != nil {
		return "", err
	}
	status, err := taskartifacts.Git(ctx, allocation, "", "status", "--porcelain=v1", "--untracked-files=all")
	if err != nil || len(status) != 0 {
		return "", fmt.Errorf("materialized checkpoint verification failed")
	}
	return allocation, nil
}

func verifyHandoffAllocation(ctx context.Context, allocation, repo, task, direction string, m taskartifacts.Manifest) error {
	info, err := os.Lstat(allocation)
	if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return fmt.Errorf("allocation identity changed")
	}
	head, format, err := taskartifacts.CheckSource(ctx, allocation)
	if err != nil || head != m.Commit || format != m.ObjectFormat {
		return fmt.Errorf("allocation checkpoint changed")
	}
	common, err := taskartifacts.Git(ctx, allocation, "", "rev-parse", "--path-format=absolute", "--git-common-dir")
	if err != nil || filepath.Clean(strings.TrimSpace(string(common))) != filepath.Clean(repo) {
		return fmt.Errorf("allocation repository changed")
	}
	branch, err := taskartifacts.Git(ctx, allocation, "", "symbolic-ref", "HEAD")
	if err != nil || strings.TrimSpace(string(branch)) != "refs/heads/wks/handoff-"+task+"-"+direction {
		return fmt.Errorf("allocation branch changed")
	}
	return nil
}

// Seven days starts at accepted disposition, never terminal-message ACK.
const handoffCleanupGrace = 7 * 24 * time.Hour

func handoffCleanupAllowed(binding taskartifacts.RepositoryBinding, rec *handoffRecord, now time.Time) error {
	if !binding.Cleanup || rec.Keep || rec.AcceptedAt == 0 || now.Before(time.UnixMilli(rec.AcceptedAt).Add(handoffCleanupGrace)) || rec.Custody == "" || rec.Result == nil || rec.State != "result-sealed" {
		return fmt.Errorf("cleanup retained: accepted disposition, seven-day grace and durable receiver custody are required")
	}
	digest, err := rec.Result.Seal()
	if err != nil || digest != rec.Custody {
		return fmt.Errorf("cleanup custody identity mismatch")
	}
	return nil
}

func (r *registry) scheduleHandoffCleanup(binding taskartifacts.RepositoryBinding, task string, acceptedAt int64) {
	if r.remote == nil {
		return
	} // The origin keeps its review custody.
	wait := time.Until(time.UnixMilli(acceptedAt).Add(handoffCleanupGrace))
	if wait < 0 {
		wait = 0
	}
	time.AfterFunc(wait, func() {
		raw, _ := json.Marshal(handoffRequest{Operation: "cleanup", Binding: binding.ID, Task: task, OriginKey: binding.Owner})
		_, _ = r.taskHandoff(context.Background(), raw)
	})
}

func (r *registry) cleanupHandoff(ctx context.Context, binding taskartifacts.RepositoryBinding, rec *handoffRecord, now time.Time) error {
	if err := handoffCleanupAllowed(binding, rec, now); err != nil {
		return err
	}
	task := rec.Plan.Input.Task
	dir := r.handoffDir(binding, task)
	if rec.Allocation != filepath.Join(dir, "input-worktree") || r.remote == nil {
		return fmt.Errorf("cleanup allocation identity mismatch")
	}
	r.remote.mu.Lock()
	d := r.remote.m[task]
	var sessionID string
	if d != nil && d.lease != nil && d.lease.Handoff != nil && d.lease.Handoff.Digest == rec.Digest && d.lease.Owner == rec.Owner && d.lease.Cwd == rec.Allocation && d.lease.Claimed && d.last != nil && d.last.Final {
		sessionID = d.sessionID
	}
	r.remote.mu.Unlock()
	if sessionID == "" {
		return fmt.Errorf("cleanup blocked: admission or worker outcome unknown")
	}
	worker, found := findFleetSession(r.fleetSessions(ctx), sessionID)
	if !found || worker.Status != "ended" {
		return fmt.Errorf("cleanup blocked: worker has not been verified stopped")
	}
	_, allocationErr := os.Lstat(rec.Allocation)
	allocationExists := allocationErr == nil
	if allocationErr != nil && (!os.IsNotExist(allocationErr) || !rec.Cleaning) {
		return fmt.Errorf("cleanup allocation missing before durable cleanup intent")
	}
	if allocationExists {
		if err := verifyRecordedAllocation(rec); err != nil {
			return err
		}
		head, _, err := taskartifacts.CheckSource(ctx, rec.Allocation)
		if err != nil || head != rec.Result.Commit {
			return fmt.Errorf("cleanup blocked: checkpoint changed or worktree dirty")
		}
		known := map[string]string{}
		for _, m := range []taskartifacts.Manifest{rec.Plan.Input, *rec.Result} {
			for _, e := range m.Entries {
				known[".workspacer/handoffs/"+task+"/"+e.Name] = e.SHA256
			}
		}
		ignored, err := taskartifacts.Git(ctx, rec.Allocation, "", "ls-files", "--others", "--ignored", "--exclude-standard", "-z")
		if err != nil {
			return err
		}
		for _, name := range strings.Split(string(ignored), "\x00") {
			if name == "" {
				continue
			}
			expected, ok := known[name]
			info, e := os.Lstat(filepath.Join(rec.Allocation, name))
			if !ok || e != nil || !info.Mode().IsRegular() || info.Size() > taskartifacts.FileBytes {
				return fmt.Errorf("cleanup blocked: unexpected ignored file")
			}
			b, e := os.ReadFile(filepath.Join(rec.Allocation, name))
			if e != nil || taskartifacts.Digest(b) != expected {
				return fmt.Errorf("cleanup blocked: artifact changed after sealing")
			}
		}
	}
	// Validate all remaining spool entries before deleting any byte. Missing
	// entries are allowed only after the durable cleanup intent was recorded.
	for direction, m := range map[string]taskartifacts.Manifest{"input": rec.Plan.Input, "result": *rec.Result} {
		folder := filepath.Join(dir, direction)
		files, err := os.ReadDir(folder)
		if err != nil {
			if os.IsNotExist(err) && rec.Cleaning {
				continue
			}
			return err
		}
		if !rec.Cleaning && len(files) != len(m.Entries) {
			return fmt.Errorf("cleanup blocked: unexpected staging files")
		}
		for _, file := range files {
			matched := false
			for i, entry := range m.Entries {
				if file.Name() != fmt.Sprintf("%d.bytes", i) {
					continue
				}
				info, err := file.Info()
				if err != nil || !info.Mode().IsRegular() || info.Size() != entry.Size {
					return fmt.Errorf("cleanup staging file changed")
				}
				b, err := os.ReadFile(filepath.Join(folder, file.Name()))
				if err != nil || taskartifacts.Digest(b) != entry.SHA256 {
					return fmt.Errorf("cleanup staging checksum changed")
				}
				matched = true
			}
			if !matched {
				return fmt.Errorf("cleanup blocked: unexpected staging file")
			}
		}
	}
	rec.Cleaning = true
	if err := saveHandoff(dir, rec); err != nil {
		return err
	}
	// Compare-delete only this binding's exact generated refs. A moved ref is
	// never force-deleted. Receiver custody survives every failure below.
	for direction, commit := range map[string]string{"input": rec.Plan.Input.Commit, "result": rec.Result.Commit} {
		ref, _ := binding.Ref(task, direction)
		if err := deleteHandoffRef(ctx, binding, filepath.Join(dir, "input-git"), commit, ref); err != nil {
			return fmt.Errorf("cleanup blocked: ref changed or approved remote unavailable")
		}
	}
	if allocationExists {
		if _, err := taskartifacts.Git(ctx, filepath.Join(dir, "input-git"), "", "worktree", "remove", "--", rec.Allocation); err != nil {
			return fmt.Errorf("cleanup blocked: worktree removal refused")
		}
	}
	// These directories contain only generated numeric files. Unexpected
	// entries block deletion instead of broad RemoveAll on a task root.
	for direction, m := range map[string]taskartifacts.Manifest{"input": rec.Plan.Input, "result": *rec.Result} {
		for i := range m.Entries {
			if err := os.Remove(filepath.Join(dir, direction, fmt.Sprintf("%d.bytes", i))); err != nil && !os.IsNotExist(err) {
				return err
			}
		}
		if err := os.Remove(filepath.Join(dir, direction)); err != nil && !os.IsNotExist(err) {
			return err
		}
	}
	rec.State = "cleaned"
	return saveHandoff(dir, rec)
}

func (r *registry) handoffConfigDir() string {
	if r.handoffRoot != "" {
		if canonical, err := filepath.EvalSymlinks(r.handoffRoot); err == nil {
			return canonical
		}
		return r.handoffRoot
	}
	return configDir()
}

func verifyRecordedAllocation(rec *handoffRecord) error {
	identity, err := taskartifacts.DirectoryIdentity(rec.Allocation)
	if err != nil || identity != rec.AllocationId || identity == "" {
		return fmt.Errorf("custody allocation identity changed")
	}
	return taskartifacts.VerifyPrivateTree(rec.Allocation)
}

func finishHandoffPin(ctx context.Context, rec *handoffRecord, dir string) error {
	if rec.Owner != "local-host" || rec.State != "pinning" || rec.SourceRepo == "" || rec.InputPin == "" {
		return fmt.Errorf("invalid source pin intent")
	}
	commit := rec.Plan.Input.Commit
	if _, err := taskartifacts.Git(ctx, rec.SourceRepo, "", "update-ref", rec.InputPin, commit, strings.Repeat("0", len(commit))); err != nil {
		got, checkErr := taskartifacts.Git(ctx, rec.SourceRepo, "", "rev-parse", "--verify", rec.InputPin)
		if checkErr != nil || strings.TrimSpace(string(got)) != commit {
			return fmt.Errorf("source pin changed or could not be retained")
		}
	}
	rec.State = "frozen"
	return saveHandoff(dir, rec)
}
