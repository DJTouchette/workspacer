package taskartifacts

import (
	"context"
	"crypto/rand"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestStorageReservationsSurviveInterruptedAdmission(t *testing.T) {
	root := t.TempDir()
	raw, _ := json.Marshal(TaskStorageBudget)
	for _, name := range []string{"one", "two", "three"} {
		dir := filepath.Join(root, name)
		if err := MakePrivateDirectory(dir); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "storage-reservation.json"), raw, 0600); err != nil {
			t.Fatal(err)
		}
	}
	status, err := InspectStorage(root)
	if err != nil {
		t.Fatal(err)
	}
	if status.Used+status.Reserved != StorageBudget {
		t.Fatal("crash reservations not counted", status)
	}
	fourth := filepath.Join(root, "four")
	if err := MakePrivateDirectory(fourth); err != nil {
		t.Fatal(err)
	}
	if release, err := ReserveStorage(root, fourth); err == nil {
		release()
		t.Fatal("overbooked retained reservations")
	}
	release, err := ReserveStorage(root, filepath.Join(root, "one"))
	if err != nil {
		t.Fatal("same task could not resume", err)
	}
	release()
	for _, name := range []string{"two", "three"} {
		if _, err := os.Stat(filepath.Join(root, name, "storage-reservation.json")); err != nil {
			t.Fatal("sole-copy reservation evicted", err)
		}
	}
}

func TestStorageLockCannotBeDoubleBooked(t *testing.T) {
	root := t.TempDir()
	release, err := lockStorage(root)
	if err != nil {
		t.Fatal(err)
	}
	if second, err := lockStorage(root); err == nil {
		second()
		release()
		t.Fatal("native admission lock double booked")
	}
	release()
	next, err := lockStorage(root)
	if err != nil {
		t.Fatal("native lock did not release", err)
	}
	next()
}

func TestStorageLimitStopsNativeGitAndRetainsChargedBytes(t *testing.T) {
	ctx := context.Background()
	source := t.TempDir()
	root := t.TempDir()
	dest := filepath.Join(root, "quarantine")
	if err := MakePrivateDirectory(dest); err != nil {
		t.Fatal(err)
	}
	git := func(dir string, args ...string) {
		t.Helper()
		if _, err := Git(ctx, dir, "", args...); err != nil {
			t.Fatal(err)
		}
	}
	git(source, "init")
	data := make([]byte, 2<<20)
	if _, err := rand.Read(data); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(source, "data.bin"), data, 0600); err != nil {
		t.Fatal(err)
	}
	git(source, "add", "data.bin")
	git(source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "commit", "-m", "quota fixture")
	head, headErr := Git(ctx, source, "", "rev-parse", "HEAD")
	if headErr != nil {
		t.Fatal(headErr)
	}
	if err := VerifyTree(ctx, source, strings.TrimSpace(string(head))); err != nil {
		t.Fatal("streamed binary tree verification failed", err)
	}
	git(dest, "init", "--bare")
	bounded := context.WithValue(ctx, storageContextKey{}, storageGuard{dest, 512 << 10})
	_, err := Git(bounded, dest, "", "-c", "protocol.file.allow=always", "fetch", "--keep", "--", source, "HEAD:refs/handoff/test")
	if err == nil || !strings.Contains(err.Error(), "storage limit reached") {
		t.Fatal("fetch did not enforce storage limit", err)
	}
	status, err := InspectStorage(root)
	if err != nil {
		t.Fatal(err)
	}
	if status.Used == 0 {
		t.Fatal("failed quarantine vanished instead of remaining charged")
	}
}
