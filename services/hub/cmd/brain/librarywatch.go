package main

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"sort"
	"time"
)

// Observe the same library projections clients can read, including external
// editor/agent writes. Publish only a change signal, never source bytes.
func (r *registry) libraryRevision(ctx context.Context) [32]byte {
	roots := append([]string{""}, r.agentCwds(ctx)...)
	sort.Strings(roots)
	hash := sha256.New()
	previous := "\x00"
	for _, cwd := range roots {
		if cwd == previous {
			continue
		}
		previous = cwd
		data, _ := json.Marshal(listLibrary(cwd, libraryFileGuardFor("library.list", cwd), libraryFilter{}))
		_, _ = hash.Write([]byte(cwd))
		_, _ = hash.Write(data)
	}
	var result [32]byte
	copy(result[:], hash.Sum(nil))
	return result
}
func (r *registry) runLibraryChanges(ctx context.Context) {
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()
	previous := r.libraryRevision(ctx)
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			next := r.libraryRevision(ctx)
			if next != previous {
				previous = next
				if r.publish != nil {
					r.publish("library.changed", json.RawMessage(`{}`))
				}
			}
		}
	}
}
