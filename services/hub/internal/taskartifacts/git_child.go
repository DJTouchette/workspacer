package taskartifacts

import (
	"github.com/djtouchette/workspacer-hub/internal/parentwatch"
	"os"
	"os/exec"
	"time"
)

const gitChildFlag = "--workspacer-task-git-child"

// A narrow host helper gives Windows a job object BEFORE Git can start a
// descendant. Parent death, timeout or quota refusal kills the complete tree.
// It is also used by native test binaries; it starts no application services.
func init() {
	if len(os.Args) < 2 || os.Args[1] != gitChildFlag {
		return
	}
	if confineGitChild() != nil {
		os.Exit(1)
	}
	parentwatch.Watch(stopGitChild)
	dir := os.Getenv("WORKSPACER_TASK_STORAGE")
	if dir == "" {
		os.Exit(1)
	}
	go func() {
		timer := time.NewTimer(20 * time.Second)
		defer timer.Stop()
		tick := time.NewTicker(20 * time.Millisecond)
		defer tick.Stop()
		for {
			select {
			case <-timer.C:
				stopGitChild()
			case <-tick.C:
				size, err := directoryBytes(dir)
				free, freeErr := freeStorageBytes(dir)
				if err != nil || freeErr != nil || size > TaskStorageBudget-2*TaskBytes || free < StorageFreeFloor {
					stopGitChild()
				}
			}
		}
	}()
	cmd := exec.Command("git", os.Args[2:]...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if cmd.Run() != nil {
		os.Exit(1)
	}
	size, err := directoryBytes(dir)
	if err != nil || size > TaskStorageBudget-2*TaskBytes {
		stopGitChild()
	}
	os.Exit(0)
}
