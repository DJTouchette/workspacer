package supervisor

import (
	"bytes"
	"sync"
)

// logWriter is the io.Writer wired to a sidecar's stdout/stderr when
// Spec.LogLines is set. It buffers bytes and, on each newline, emits the
// complete line (trailing '\n' stripped) via s.emitLog(stream, line) so the
// output reaches the bus as discrete plugin.log events. Modeled on the
// launcher's prefixWriter (cmd/workspacer/child.go), but its sink is the bus
// rather than another io.Writer.
//
// Write is called concurrently by Go's separate stdout and stderr copier
// goroutines (a distinct logWriter per stream), so the buffer is mutex-guarded.
type logWriter struct {
	sup    *Supervisor
	stream string // "stdout" | "stderr"

	mu  sync.Mutex
	buf []byte
}

// newLogWriter creates a logWriter that publishes s's output for one stream.
func newLogWriter(s *Supervisor, stream string) *logWriter {
	return &logWriter{sup: s, stream: stream}
}

func (w *logWriter) Write(b []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.buf = append(w.buf, b...)
	for {
		i := bytes.IndexByte(w.buf, '\n')
		if i < 0 {
			break
		}
		w.sup.emitLog(w.stream, string(w.buf[:i]))
		w.buf = w.buf[i+1:]
	}
	return len(b), nil
}

// flush emits any buffered final line (a child that died mid-line still gets its
// last words on the bus). An empty trailing buffer emits nothing.
func (w *logWriter) flush() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if len(w.buf) > 0 {
		w.sup.emitLog(w.stream, string(w.buf))
		w.buf = nil
	}
}
