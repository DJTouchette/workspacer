package main

// Portable file-change observation for headless clients. Stat polling retains
// the watch across atomic replacement/deletion, including files on network
// mounts where native notifications are not reliable. No file bytes are read.
import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"sync"
	"time"
)

type watchedFile struct {
	refs   int
	info   os.FileInfo
	leases map[string]time.Time
}
type fileWatchState struct {
	mu    sync.Mutex
	paths map[string]*watchedFile
}

func (r *registry) fsWatch(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Path    string `json:"path"`
		WatchID string `json:"watchId"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if len(p.WatchID) > 128 {
		return nil, fmt.Errorf("watchId is too long")
	}
	path, err := assertPathAllowed("fs.watch", p.Path, r.workspaceRoots(ctx))
	if err != nil {
		return nil, err
	}
	if r.publish == nil {
		return nil, fmt.Errorf("file change publisher is unavailable")
	}
	info, err := os.Stat(path)
	if err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	r.fileWatches.mu.Lock()
	defer r.fileWatches.mu.Unlock()
	if r.fileWatches.paths == nil {
		r.fileWatches.paths = map[string]*watchedFile{}
	}
	if entry := r.fileWatches.paths[path]; entry != nil {
		if p.WatchID != "" {
			if len(entry.leases) >= 128 {
				if _, ok := entry.leases[p.WatchID]; !ok {
					return nil, fmt.Errorf("too many file watchers")
				}
			}
			entry.leases[p.WatchID] = time.Now().Add(3 * time.Minute)
		} else {
			entry.refs++
		}
	} else {
		if len(r.fileWatches.paths) >= 1024 {
			return nil, fmt.Errorf("too many watched files")
		}
		entry := &watchedFile{info: info, leases: map[string]time.Time{}}
		if p.WatchID != "" {
			entry.leases[p.WatchID] = time.Now().Add(3 * time.Minute)
		} else {
			entry.refs = 1
		}
		r.fileWatches.paths[path] = entry
	}
	return json.Marshal(map[string]any{"ok": true, "path": path})
}

func (r *registry) fsUnwatch(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Path    string `json:"path"`
		WatchID string `json:"watchId"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	path, err := assertPathAllowed("fs.unwatch", p.Path, r.workspaceRoots(ctx))
	if err != nil {
		return nil, err
	}
	r.fileWatches.mu.Lock()
	defer r.fileWatches.mu.Unlock()
	if entry := r.fileWatches.paths[path]; entry != nil {
		if p.WatchID != "" {
			delete(entry.leases, p.WatchID)
		} else if entry.refs > 0 {
			entry.refs--
		}
		if entry.refs <= 0 && len(entry.leases) == 0 {
			delete(r.fileWatches.paths, path)
		}
	}
	return json.Marshal(map[string]any{"ok": true})
}

func (r *registry) pollFileChanges(ctx context.Context) {
	r.fileWatches.mu.Lock()
	paths := make([]string, 0, len(r.fileWatches.paths))
	for path, entry := range r.fileWatches.paths {
		for id, expires := range entry.leases {
			if time.Now().After(expires) {
				delete(entry.leases, id)
			}
		}
		if entry.refs <= 0 && len(entry.leases) == 0 {
			delete(r.fileWatches.paths, path)
			continue
		}
		paths = append(paths, path)
	}
	r.fileWatches.mu.Unlock()
	if len(paths) == 0 {
		return
	}
	roots := r.workspaceRoots(ctx)
	for _, path := range paths {
		// Recheck roots and symlinks before observing. A removed project or a path
		// replaced by an escaping link must not turn into an outside-file oracle.
		checked, err := assertPathAllowed("fs.watch", path, roots)
		if err != nil || checked != path {
			r.fileWatches.mu.Lock()
			delete(r.fileWatches.paths, path)
			r.fileWatches.mu.Unlock()
			continue
		}
		info, err := os.Stat(path)
		if err != nil && !os.IsNotExist(err) {
			continue
		}
		kind := ""
		r.fileWatches.mu.Lock()
		if entry := r.fileWatches.paths[path]; entry != nil {
			old := entry.info
			if (old == nil) != (info == nil) || (old != nil && info != nil && !os.SameFile(old, info)) {
				kind = "rename"
			} else if old != nil && info != nil && (old.Size() != info.Size() || old.ModTime() != info.ModTime() || old.Mode() != info.Mode()) {
				kind = "change"
			}
			entry.info = info
		}
		r.fileWatches.mu.Unlock()
		if kind != "" && r.publish != nil {
			payload, _ := json.Marshal(map[string]string{"path": path, "eventType": kind})
			r.publish("fs.changed", payload)
		}
	}
}

func (r *registry) runFileChanges(ctx context.Context) {
	tick := time.NewTicker(500 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			r.pollFileChanges(ctx)
		}
	}
}
