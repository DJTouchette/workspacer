package taskartifacts

import (
	"encoding/json"
	"os"
	"path/filepath"
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
