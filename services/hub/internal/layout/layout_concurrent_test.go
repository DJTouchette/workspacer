package layout

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/broker"
)

func TestConcurrentSetPersistsMonotonicOnDisk(t *testing.T) {
	path := filepath.Join(t.TempDir(), "layout.json")
	s := New(broker.New(), path)

	var stop atomic.Bool
	var regressed atomic.Int64
	regressed.Store(-1)
	var corrupt atomic.Int64
	corrupt.Store(-1)

	// Reader continuously observes the persisted file. Every accepted write
	// bumps Version under the lock, so on disk the version must never go
	// backwards. If persist() runs outside the lock (racing on the shared
	// s.path+".tmp"), an older-version write can land last and the reader will
	// observe the version regress — exactly the silent layout-revert-on-restart
	// bug.
	var rwg sync.WaitGroup
	rwg.Add(1)
	go func() {
		defer rwg.Done()
		last := 0
		for !stop.Load() {
			raw, err := os.ReadFile(path)
			if err != nil || len(raw) == 0 {
				continue
			}
			var d Document
			if err := json.Unmarshal(raw, &d); err != nil {
				corrupt.Store(int64(len(raw)))
				continue
			}
			if d.Version < last {
				regressed.CompareAndSwap(-1, int64(last))
			}
			if d.Version > last {
				last = d.Version
			}
		}
	}()

	const writers = 8
	const rounds = 300
	var wg sync.WaitGroup
	for i := 0; i < writers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for r := 0; r < rounds; r++ {
				if _, err := s.Set(json.RawMessage(`{"data":{"x":1}}`)); err != nil {
					t.Errorf("Set: %v", err)
					return
				}
			}
		}()
	}
	wg.Wait()
	stop.Store(true)
	rwg.Wait()

	if v := regressed.Load(); v >= 0 {
		t.Fatalf("on-disk layout version regressed below %d (older write landed last on disk)", v)
	}
	if v := corrupt.Load(); v >= 0 {
		t.Fatalf("on-disk layout file was corrupt (%d bytes of invalid JSON from shared .tmp race)", v)
	}
}
