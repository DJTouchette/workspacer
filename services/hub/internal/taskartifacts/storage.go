package taskartifacts

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
)

// Leaves room for the OS, provider and bounded write overshoot on a 10 GB
// worker. Retained and failed transfers count; receipts never evict sole copies.
const StorageBudget int64 = 6 << 30
const TaskStorageBudget int64 = 2 << 30
const StorageFreeFloor uint64 = 2 << 30
const GitPackLimit uint64 = 512 << 20

var storageMu sync.Mutex

type storageContextKey struct{}
type storageGuard struct {
	dir   string
	limit int64
}
type StorageStatus struct {
	Used      int64  `json:"usedBytes"`
	Reserved  int64  `json:"reservedBytes"`
	Limit     int64  `json:"limitBytes"`
	TaskLimit int64  `json:"taskLimitBytes"`
	Retention string `json:"retention"`
}

func directoryBytes(dir string) (int64, error) {
	var total int64
	var count int
	err := filepath.WalkDir(dir, func(name string, e os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		count++
		if count > 200000 {
			return fmt.Errorf("storage entry limit exceeded")
		}
		if e.Type()&os.ModeSymlink != 0 {
			return fmt.Errorf("quarantine storage link refused")
		}
		if e.IsDir() {
			return nil
		}
		info, err := e.Info()
		if err != nil {
			return err
		}
		if !info.Mode().IsRegular() {
			return fmt.Errorf("quarantine special file refused")
		}
		total += info.Size()
		return nil
	})
	return total, err
}

func InspectStorage(root string) (StorageStatus, error) {
	status := StorageStatus{Limit: StorageBudget, TaskLimit: TaskStorageBudget, Retention: "Git history stays retained and counts against the storage limit. Automatic cleanup removes accepted execution worktrees and report copies after the grace period."}
	used, err := directoryBytes(root)
	if os.IsNotExist(err) {
		return status, nil
	}
	if err != nil {
		return status, err
	}
	status.Used = used
	err = filepath.WalkDir(root, func(name string, e os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if e.Name() != "storage-reservation.json" || e.IsDir() {
			return nil
		}
		raw, err := os.ReadFile(name)
		if err != nil {
			return err
		}
		var n int64
		if len(raw) > 64 || json.Unmarshal(raw, &n) != nil || n != TaskStorageBudget {
			return fmt.Errorf("invalid storage reservation; retain for reconciliation")
		}
		used, err := directoryBytes(filepath.Dir(name))
		if err != nil {
			return err
		}
		if n > used {
			status.Reserved += n - used
		}
		return nil
	})
	return status, err
}

// The brain is the sole disk owner. Serialization covers admission through
// release; a crash leaves the durable reservation charged until same-task retry.
func ReserveStorage(root, dir string) (func(), error) {
	storageMu.Lock()
	fail := func(err error) (func(), error) { storageMu.Unlock(); return nil, err }
	status, err := InspectStorage(root)
	if err != nil {
		return fail(err)
	}
	used, err := directoryBytes(dir)
	if err != nil {
		return fail(err)
	}
	ownHeld := int64(0)
	marker := filepath.Join(dir, "storage-reservation.json")
	if _, err := os.Stat(marker); err == nil && used < TaskStorageBudget {
		ownHeld = TaskStorageBudget - used
	}
	if used > TaskStorageBudget || status.Used+status.Reserved-ownHeld+TaskStorageBudget-used > StorageBudget {
		return fail(fmt.Errorf("quarantine storage admission refused: retained bytes and reservations exceed 6 GiB; no sole copies removed"))
	}
	free, err := freeStorageBytes(root)
	if err != nil {
		return fail(err)
	}
	needed := TaskStorageBudget - used
	if free < StorageFreeFloor+uint64(needed) {
		return fail(fmt.Errorf("quarantine storage admission refused: preserve 2 GiB free disk reserve"))
	}
	raw, _ := json.Marshal(TaskStorageBudget)
	f, err := os.OpenFile(marker, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0600)
	if err != nil {
		return fail(err)
	}
	_, err = f.Write(raw)
	if err == nil {
		err = f.Sync()
	}
	closeErr := f.Close()
	if err != nil {
		return fail(err)
	}
	if closeErr != nil {
		return fail(closeErr)
	}
	return func() { _ = os.Remove(marker); storageMu.Unlock() }, nil
}

func WithStorageLimit(ctx context.Context, dir string) context.Context {
	return context.WithValue(ctx, storageContextKey{}, storageGuard{dir, TaskStorageBudget - 2*TaskBytes})
}
