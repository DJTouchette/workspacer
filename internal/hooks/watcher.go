// Package hooks watches for Claude Code hook events from a JSONL file.
package hooks

import (
	"bufio"
	"encoding/json"
	"log"
	"os"
	"path/filepath"
	"time"
)

// Watcher tails a JSONL events file for new hook events.
type Watcher struct {
	filePath string
	OnEvent  func(event map[string]any)
	stop     chan struct{}
}

// NewWatcher creates a watcher for the events file in the given data directory.
func NewWatcher(dataDir string) *Watcher {
	os.MkdirAll(dataDir, 0755)
	return &Watcher{
		filePath: filepath.Join(dataDir, "events.jsonl"),
		stop:     make(chan struct{}),
	}
}

// Start begins watching the events file for new lines.
func (w *Watcher) Start() {
	// Create file if it doesn't exist
	if _, err := os.Stat(w.filePath); os.IsNotExist(err) {
		os.WriteFile(w.filePath, []byte{}, 0644)
	}

	// Start at the end of the file
	fi, err := os.Stat(w.filePath)
	if err != nil {
		log.Printf("[hooks] cannot stat events file: %v", err)
		return
	}
	offset := fi.Size()

	log.Printf("[hooks] watching %s (offset=%d)", w.filePath, offset)

	go func() {
		for {
			select {
			case <-w.stop:
				return
			default:
			}

			// At the start of each poll cycle, check file size
			fi, err := os.Stat(w.filePath)
			if err == nil && fi.Size() > 50*1024*1024 {
				os.Rename(w.filePath, w.filePath+".old")
				os.WriteFile(w.filePath, []byte{}, 0644)
				offset = 0
			}

			f, err := os.Open(w.filePath)
			if err != nil {
				time.Sleep(time.Second)
				continue
			}

			// If the file was truncated/rotated, reset offset
			fi, _ = f.Stat()
			if fi != nil && fi.Size() < offset {
				log.Printf("[hooks] events file truncated, resetting offset")
				offset = 0
			}

			// Seek to where we left off
			f.Seek(offset, 0)
			scanner := bufio.NewScanner(f)
			scanner.Buffer(make([]byte, 1024*1024), 1024*1024)

			newOffset := offset
			for scanner.Scan() {
				line := scanner.Bytes()
				newOffset += int64(len(line)) + 1 // +1 for newline
				if len(line) == 0 {
					continue
				}

				var event map[string]any
				if err := json.Unmarshal(line, &event); err != nil {
					continue
				}

				if w.OnEvent != nil {
					w.OnEvent(event)
				}
			}
			offset = newOffset
			f.Close()

			// Poll interval
			time.Sleep(200 * time.Millisecond)
		}
	}()
}

// Stop stops the watcher.
func (w *Watcher) Stop() {
	close(w.stop)
}
