package routing

// THE DECISION LOG: an append-only record of what routing answered, and of what
// the spawn gate did with the answer.
//
// It exists for three reasons, in ascending order of how long they matter.
//
//  1. AUDIT. A routing decision commits an hour of a frontier model's allowance,
//     and a ceiling clamp silently taking a model away would be a downgrade only
//     a server log knew about. Both are written here, with the reasoning, so
//     "why did this worker come up on Terra" has an answer tomorrow.
//
//  2. THE JOIN. Every decision carries a decisionId, and a spawn that acts on
//     one carries the same id on the wire (`decisionId` in agents.spawn). That
//     is what makes "the decision, and the worker it actually produced" one row
//     instead of two guesses — including on a headless node, which records no
//     analytics at all (workspacer.db is written by the Electron main process).
//
//  3. CALIBRATION. routing.yaml's `forecast_weights:` are unitless today: an
//     implementation counts 4 and a review counts 2, and nothing turns that into
//     a share of an allowance. The honest fix is measurement, and measurement
//     needs a record of what was decided next to what it cost. This file is that
//     record's first half.
//
// WHERE AND HOW. Beside jobs.json in the hub's own 0600 state directory, which
// is where routing.yaml already lives and is refused to fs.write — the same
// placement argument, for a file that names project directories and the models
// spent in them. JSONL rather than one JSON document because it is APPEND-ONLY:
// a document has to be read, re-marshalled and rewritten to grow by one row, so
// a crash mid-write loses the history, and two writers racing lose one of them.
// One O_APPEND write of one line loses nothing and needs no read.
//
// SIZE IS CAPPED BY ROTATION, not by trimming. When the live file passes the
// cap it is renamed to <name>.1 (replacing whatever was there) and a fresh file
// starts. Trimming from the front would mean reading and rewriting the whole
// file, which is the thing this format exists to avoid, and truncating in place
// would leave a half-line at the seam.
//
// NOTHING HERE IS FATAL. A directory that cannot be created, a disk that is
// full, a permission error: the log complains once and routing keeps deciding.
// A hub that refused to route because it could not write its audit trail would
// be trading the feature for the record of the feature.

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"log"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// DefaultDecisionLogMaxBytes is when the live file rotates. Roughly a hundred
// thousand decisions at the size these rows run, which on a fleet deciding once
// a minute is months — and two generations are kept, so the window is longer
// still.
const DefaultDecisionLogMaxBytes int64 = 8 << 20

// DecisionLogPathFor is the log that belongs beside a given routing.yaml. The
// two files are a pair: the matrix is the policy, this is what the policy did.
func DecisionLogPathFor(matrixPath string) string {
	if matrixPath == "" {
		return ""
	}
	return filepath.Join(filepath.Dir(matrixPath), "routing-decisions.jsonl")
}

// NewDecisionID mints the id a decision is recorded under and a spawn quotes
// back.
//
// Random rather than derived from the decision's own contents, and that is the
// point: two identical asks a second apart are two decisions, they may be
// answered differently (the whole feature is that capacity moves), and an id
// that collapsed them would silently join a spawn to the wrong one. Random
// rather than a counter for the same reason the log is append-only — a counter
// is process state, and it restarts.
func NewDecisionID() string {
	var b [12]byte
	if _, err := rand.Read(b[:]); err != nil {
		// Cannot realistically fail. If it ever does, an empty id is the honest
		// answer: it says "this decision has no join key" rather than handing out
		// one that repeats.
		return ""
	}
	return "rd_" + hex.EncodeToString(b[:])
}

// LogKind is the closed vocabulary of rows in the log.
type LogKind string

const (
	// KindDecision is one answer from routing.select.
	KindDecision LogKind = "decision"
	// KindSpawn is one agents.spawn as the enforcement site saw it: the decision
	// it quoted, and what the ceiling took away from it, if anything.
	KindSpawn LogKind = "spawn"
)

// Entry is one line of the log. Kind decides which of the two payloads is set;
// both are omitted when empty so a row is readable by eye.
type Entry struct {
	Kind LogKind `json:"kind"`
	// At is the wall clock, RFC3339. The hub's own, not the caller's.
	At time.Time `json:"at"`
	// DecisionID joins the two kinds. Present on both.
	DecisionID string `json:"decisionId,omitempty"`

	Decision *Decision   `json:"decision,omitempty"`
	Spawn    *SpawnEntry `json:"spawn,omitempty"`
}

// SpawnEntry is what the spawn gate saw and did. Deliberately NOT the whole
// params object: a spawn's first message is the user's prose and its profileId
// names an account, and neither belongs in a log whose job is "which capability
// went where".
type SpawnEntry struct {
	// Role / Capability are what the spawn declared, before any clamp.
	Role       string `json:"role,omitempty"`
	Capability string `json:"capability,omitempty"`
	// Cwd is the CANONICAL directory the ceiling was looked up on — the resolved
	// one, not the caller's spelling, because the resolved one is what the
	// lookup actually used.
	Cwd       string `json:"cwd,omitempty"`
	Provider  string `json:"provider,omitempty"`
	Model     string `json:"model,omitempty"`
	Effort    string `json:"effort,omitempty"`
	ToolScope string `json:"toolScope,omitempty"`
	// Caller identifies the connection, by credential fingerprint and tier —
	// never by token.
	CallerScope   string `json:"callerScope,omitempty"`
	CallerTokenID string `json:"callerTokenId,omitempty"`
	// Ceiling is the verdict, present whether or not it refused anything: "we
	// looked, and the ceiling allowed it" is a record worth having.
	Ceiling *CeilingVerdict `json:"ceiling,omitempty"`
	// Scrubbed lists the params the gate removed or rewrote.
	Scrubbed []string `json:"scrubbed,omitempty"`
}

// DecisionLog appends rows. Safe for concurrent use: routing.select answers off
// the read loop and the spawn gate runs on the router's dispatch path, so two
// writers is the ordinary case rather than the exotic one.
type DecisionLog struct {
	mu       sync.Mutex
	path     string
	maxBytes int64
	// writeErrLogged keeps a broken disk from logging once per decision. It
	// resets on the first successful write.
	writeErrLogged bool
}

// NewDecisionLog opens (lazily — nothing is created until the first row) the log
// at path. path "" disables it entirely, which is what a test and a read-only
// deployment want, and what --routing-file "" already means for the matrix.
func NewDecisionLog(path string, maxBytes int64) *DecisionLog {
	if maxBytes <= 0 {
		maxBytes = DefaultDecisionLogMaxBytes
	}
	return &DecisionLog{path: path, maxBytes: maxBytes}
}

// Path is the file this log writes ("" when disabled).
func (l *DecisionLog) Path() string {
	if l == nil {
		return ""
	}
	return l.path
}

// Decision records one routing answer. Nil receiver and empty path are no-ops,
// so a caller never has to ask whether logging is on.
func (l *DecisionLog) Decision(d Decision) {
	if l == nil || l.path == "" {
		return
	}
	l.append(Entry{Kind: KindDecision, At: time.Now(), DecisionID: d.DecisionID, Decision: &d})
}

// Spawn records one spawn as the enforcement site saw it.
func (l *DecisionLog) Spawn(decisionID string, s SpawnEntry) {
	if l == nil || l.path == "" {
		return
	}
	l.append(Entry{Kind: KindSpawn, At: time.Now(), DecisionID: decisionID, Spawn: &s})
}

func (l *DecisionLog) append(e Entry) {
	line, err := json.Marshal(e)
	if err != nil {
		return
	}
	line = append(line, '\n')

	l.mu.Lock()
	defer l.mu.Unlock()

	if dir := filepath.Dir(l.path); dir != "" && dir != "." {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			l.complain("cannot create %s for the decision log: %v", dir, err)
			return
		}
	}
	l.rotateLocked(int64(len(line)))

	// 0600 for the same reason routing.yaml and jobs.json are: this file names
	// project directories, the models spent in them, and which credential asked.
	f, err := openDecisionLogFile(l.path)
	if err != nil {
		l.complain("cannot open the decision log %s: %v", l.path, err)
		return
	}
	defer f.Close()
	// The Unix opener's mode only applies when it creates a new file, and on
	// Windows a Go mode is not an ACL at all. Repair an existing loose file and
	// install an owner-only Windows DACL before any sensitive row is written. If
	// that cannot be established, fail closed for the audit bytes (routing itself is
	// still deliberately non-fatal).
	if err := secureDecisionLogFile(f); err != nil {
		l.complain("cannot make the decision log %s private: %v", l.path, err)
		return
	}
	if _, err := f.Write(line); err != nil {
		l.complain("cannot write to the decision log %s: %v", l.path, err)
		return
	}
	l.writeErrLogged = false
}

// rotateLocked renames the live file out of the way once it would pass the cap.
// The check is done BEFORE the write rather than after, so the cap bounds the
// file rather than being the point it is already past.
func (l *DecisionLog) rotateLocked(incoming int64) {
	st, err := os.Stat(l.path)
	if err != nil || st.Size()+incoming <= l.maxBytes {
		return
	}
	prev := l.path + ".1"
	// A pre-existing log may have been loosened between writes. Privacy must
	// follow the bytes into the retained generation, so repair the live file
	// before renaming it. If that cannot be done, keep it in place and let the
	// append path below refuse to add another sensitive row.
	if err := secureDecisionLogPath(l.path); err != nil {
		l.complain("cannot make the decision log private before rotation %s: %v", l.path, err)
		return
	}
	if err := os.Rename(l.path, prev); err != nil {
		l.complain("cannot rotate the decision log %s: %v", l.path, err)
		return
	}
	log.Printf("[routing] decision log passed %d bytes; previous generation kept at %s", l.maxBytes, prev)
}

func secureDecisionLogPath(path string) error {
	f, err := openDecisionLogFile(path)
	if err != nil {
		return err
	}
	defer f.Close()
	return secureDecisionLogFile(f)
}

func (l *DecisionLog) complain(format string, args ...any) {
	if l.writeErrLogged {
		return
	}
	l.writeErrLogged = true
	log.Printf("[routing] "+format+" — routing keeps deciding; only the audit trail is lost", args...)
}
