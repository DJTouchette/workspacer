package main

// `brief.append` — the atomic inspect-then-edit primitive for a project's
// `.workspacer/brief.md`, headless.
//
// TWIN: apps/desktop/src/main/services/briefService.ts (the implementation) and
// the brief.append registration in hubCapabilities.ts (the confinement).
//
// WHY IT IS HERE. A Fleet Manager's brief IS its memory across restarts. On a
// headless node the manager had no way to write one: brief.append is
// desktop-main-only, so the /checkpoint workflow's write path simply had no
// provider, and the fallback — fs.read + fs.write — reopens the exact race this
// primitive was built to close. A manager updates a project brief at precisely
// the moment a worker in that project is editing it, because the trigger for
// both is the same worker finishing.
//
// THE GUARANTEE, unchanged from the twin:
//
//  1. STRICTLY ADDITIVE. The output is the input with exactly ONE line
//     inserted. Nothing is rewritten, reordered or reflowed — the user's own
//     edits are authoritative. TestAppendToBriefIsStrictlyAdditive asserts that
//     removing the inserted line reproduces the input byte for byte.
//  2. SERIALIZED against itself, in-process and cross-process, by the same
//     O_EXCL advisory lock configlock.go uses, spanning read→compute→write.
//  3. COMPARE-AND-SWAP against everyone else. An agent's Edit tool does not
//     take our lock, so "nobody changed it while I was computing" is checked,
//     not assumed.
//
// WHAT IS DELIBERATELY ABSENT: brief.archive. It is not the same shape one verb
// over — it SPLICES ENTRIES OUT of a section and moves them into an archive
// file, which needs a faithful port of the board's document model (parseBrief's
// multi-line entries and `###` sub-groups, removeEntryLines, appendToArchive).
// A subtly wrong port there does not fail loudly; it silently drops entries out
// of a user's brief, which is the one damage an additive-only tool is designed
// never to do. The half a manager's memory depends on is the write, and that is
// what this file provides; archive stays a declared gap in
// headless_completeness_test.go rather than being approximated.
//
// brief.check IS ABSENT FOR THE SAME REASON, one step milder. It reads `## Now`
// as ENTRIES — a bullet plus its continuation lines, under a `##` or a `###`,
// with the board's exact boundary rules (shared/briefBoard.ts parseBrief) — and
// that document model is precisely what brief.archive's paragraph above says is
// too easy to port subtly wrong. A wrong boundary here does not corrupt anything
// (this verb only ever reports), but it produces a checker that flags live lines
// and misses dead ones, and a checker a manager stops trusting is worse than no
// checker at all: it still costs a tool call. It is not a headlessGaps entry for
// brief.archive's reason either — no shipped client calls it (it is an
// agent-facing MCP tool), so TestHeadlessGapsAreReachableFromAShippedClient
// would refuse the entry. Recorded here instead.
//
// WHAT IS DELIBERATELY PRESENT, by contrast: brief.append's append-from-result
// params (`sessionId`, `result`). Composing that line is a pure string function
// over the caller's own arguments — no session store, no facade, no worktree —
// so declining it would have been an excuse rather than a reason, and it would
// have left a headless manager writing exactly the mistranscribed `session:`
// references the feature exists to eliminate. See briefresult.go.
//
// CONFINEMENT. The caller's ONE path input is `project`, and it is held by
// fsguard.go's assertPathAllowed over the same workspaceRoots() fs.write takes
// — then the brief path is composed under the CANONICAL directory the guard
// returned, never under the caller's string. Resolving the guard's answer
// rather than re-joining the request is what stops a symlinked project dir from
// being re-interpreted after the check (BINDING DECISION 2, the same rule
// git.go's guardGitCwd states). The caller never names a file: both path
// components are literals in this file, so this reaches strictly less than
// fs.write does within the same root.
//
// AND THAT LAST SENTENCE IS NOT THE WHOLE GUARD, which is the part the desktop
// twin gets wrong. "The caller cannot name a file" bounds the BASENAME. It says
// nothing about the DIRECTORIES composed on the way to it: `project` can be a
// legitimate allowed directory while `<project>/.workspacer` is a symlink
// pointing out of every root, and a guard that only ever resolved `project`
// answers yes — truthfully — about a path that is not the one being opened. The
// composed path is therefore asserted too; see briefAppendCall.

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
	"unicode/utf8"
)

// briefSections are the four headings a brief has. `User` is fleet-brief-only
// but is accepted on any brief: refusing it would make the fleet brief a
// special case at every call site.
// TWIN: BRIEF_SECTIONS.
var briefSections = []string{"Now", "Direction", "Recently", "User"}

// briefPrependSections are written newest-first.
// TWIN: PREPEND_SECTIONS in shared/briefBoard.ts.
var briefPrependSections = map[string]bool{"recently": true}

const (
	// briefDirName / briefFileName compose BRIEF_RELATIVE_PATH. Kept as separate
	// literals because they are the two path components the caller does NOT
	// supply, and that is the whole confinement argument.
	briefDirName  = ".workspacer"
	briefFileName = "brief.md"

	// briefLineMax REFUSES rather than truncates. The tool is additive-only, so
	// it cannot go back and repair a line it damaged: a truncated write loses
	// the tail permanently, a refused write loses nothing because the caller
	// still holds every character it composed. TWIN: BRIEF_LINE_MAX.
	briefLineMax = 4000

	// Lock parameters. Generous next to configlock's, for the twin's reason:
	// this runs on a bus capability, and losing a brief line because another
	// append was mid-flight is worse than a caller waiting a beat.
	briefLockWait  = 3 * time.Second
	briefLockStale = 15 * time.Second
	// briefCASAttempts is how many times an outside writer may beat us before
	// the call is refused. TWIN: CAS_ATTEMPTS.
	briefCASAttempts = 5
)

// briefPathFor resolves a project directory to its brief path.
func briefPathFor(projectDir string) string {
	return filepath.Join(projectDir, briefDirName, briefFileName)
}

// parseBriefSection resolves a caller's section name case-insensitively.
//
// REFUSED rather than defaulted: a typo'd section silently creating a `## Nwo`
// heading in the user's brief is exactly the damage this tool must not do.
func parseBriefSection(name string) (string, error) {
	wanted := strings.ToLower(strings.TrimSpace(name))
	for _, s := range briefSections {
		if strings.ToLower(s) == wanted {
			return s, nil
		}
	}
	return "", fmt.Errorf("brief.append: unknown section %q — expected one of %s", name, strings.Join(briefSections, ", "))
}

// normalizeBriefLine turns a caller's line into one brief bullet: single-line,
// bulleted, capped.
//
// Newlines are FLATTENED rather than refused — a manager composing a sentence
// with a stray wrap should not get an error, and a multi-line insert would
// break the "exactly one line" guarantee everything else rests on.
//
// Interior SPACES are left alone, deliberately, and this is the one place where
// the obvious simplification is wrong: collapsing all whitespace also eats the
// double space in the doctrine's own dated-log format (`- YYYY-MM-DD  <what
// happened>`), so the tool that exists to write those lines would be the one
// thing that could not write one. TWIN: normalizeBriefLine.
//
// The length is measured AFTER flattening, so a caller whose sentence arrived
// wrapped is judged on what will actually be written. Measured in CHARACTERS,
// not bytes, matching the twin's String.length semantics closely enough that a
// line accepted by one provider is accepted by the other.
func normalizeBriefLine(line string) (string, error) {
	flat := flattenBriefLine(line)
	if flat == "" {
		return "", errors.New("brief.append: line is empty")
	}
	if n := utf8.RuneCountInString(flat); n > briefLineMax {
		return "", fmt.Errorf("brief.append: the line is %d characters and the limit is %d. "+
			"Nothing was written. Split it into separate entries and append each one, "+
			"or shorten it: this tool can only add a line, so it cannot repair one it cut.", n, briefLineMax)
	}
	if strings.HasPrefix(flat, "- ") || strings.HasPrefix(flat, "#") {
		return flat, nil
	}
	return "- " + flat, nil
}

// The twin's two replaces, verbatim as Go regexps: a newline run with the
// horizontal whitespace hugging it becomes ONE space, then any remaining
// tab/formfeed/vertical-tab run becomes one space. Plain spaces are matched by
// neither, which is the point — see normalizeBriefLine.
var (
	briefNewlineRun = regexp.MustCompile(`[ \t\f\v]*[\r\n]+[ \t\f\v]*`)
	briefTabRun     = regexp.MustCompile(`[\t\f\v]+`)
)

func flattenBriefLine(line string) string {
	flat := briefNewlineRun.ReplaceAllString(line, " ")
	flat = briefTabRun.ReplaceAllString(flat, " ")
	return strings.TrimSpace(flat)
}

// isBriefHeadingFor reports whether `line` is the heading for `section`,
// tolerating trailing whitespace and any number of leading hashes (briefs in
// the wild use `##`; be liberal in reading).
func isBriefHeadingFor(line, section string) bool {
	title, ok := briefHeadingTitle(line)
	return ok && strings.EqualFold(title, section)
}

// isBriefHeading reports whether `line` is any heading — where a section's body
// ends.
func isBriefHeading(line string) bool {
	title, ok := briefHeadingTitle(line)
	return ok && title != ""
}

// briefHeadingTitle parses `#{1,6}\s+(.*?)\s*$`.
func briefHeadingTitle(line string) (string, bool) {
	hashes := 0
	for hashes < len(line) && line[hashes] == '#' {
		hashes++
	}
	if hashes < 1 || hashes > 6 || hashes >= len(line) {
		return "", false
	}
	rest := line[hashes:]
	if rest[0] != ' ' && rest[0] != '\t' {
		return "", false
	}
	return strings.TrimSpace(rest), true
}

// appendToBrief is the pure core: `content` with `line` inserted into
// `section`. Exported to the tests so the additive guarantee can be checked
// without a filesystem. TWIN: appendToBrief.
func appendToBrief(content, section, bullet string) string {
	if strings.TrimSpace(content) == "" {
		// A fresh brief, ordered as the doctrine describes the sections, with the
		// caller's line in its own. This is the ONE case that writes lines the
		// caller did not supply — there is nothing here to preserve.
		var parts []string
		for _, s := range briefSections {
			body := ""
			if s == section {
				body = bullet + "\n"
			}
			parts = append(parts, "## "+s+"\n"+body)
		}
		return strings.Join(parts, "\n")
	}

	// Split KEEPING the exact line endings: a brief written on Windows, or one
	// whose last line has no trailing newline, must come back the way it went in.
	lines := strings.Split(content, "\n")
	headingAt := -1
	for i, l := range lines {
		if isBriefHeadingFor(l, section) {
			headingAt = i
			break
		}
	}
	if headingAt < 0 {
		// No such section. Append it, separated from whatever precedes it,
		// without touching a single existing line.
		trailing := "\n"
		if lines[len(lines)-1] == "" {
			trailing = ""
		}
		return content + trailing + "\n## " + section + "\n" + bullet + "\n"
	}

	// The section body runs to the next heading (or EOF).
	end := len(lines)
	for i := headingAt + 1; i < len(lines); i++ {
		if isBriefHeading(lines[i]) {
			end = i
			break
		}
	}

	at := 0
	if briefPrependSections[strings.ToLower(section)] {
		// Newest first: straight after the heading, but below a blank line the
		// author put there for spacing (inserting above it would move their
		// blank).
		at = headingAt + 1
		for at < end && strings.TrimSpace(lines[at]) == "" {
			at++
		}
	} else {
		// End of the section: after its last non-blank line, so trailing blank
		// separators before the next heading stay where the author put them.
		at = end
		for at > headingAt+1 && strings.TrimSpace(lines[at-1]) == "" {
			at--
		}
	}
	out := make([]string, 0, len(lines)+1)
	out = append(out, lines[:at]...)
	out = append(out, bullet)
	out = append(out, lines[at:]...)
	return strings.Join(out, "\n")
}

// briefSizeReport is what a section looks like after a write, so a caller
// learns the brief is over budget WITHOUT reading it.
//
// The concrete failure it exists for: a manager could not read a repo's `##
// Recently` at all, because the entries had grown past a read cap, and it found
// out by failing. TWIN: BriefSizeReport.
//
// `entriesInSection` counts the section's BULLET lines. The desktop counts the
// board parser's entries, which fold a bullet's continuation lines into it —
// the same number for the one-line entries this tool writes, and an
// over-estimate only for a hand-written multi-line entry. Named rather than
// hidden, because a counter that silently means something different from its
// twin is worse than one that is documented to approximate.
type briefSizeReport struct {
	EntriesInSection int `json:"entriesInSection"`
	BytesInSection   int `json:"bytesInSection"`
	BytesInBrief     int `json:"bytesInBrief"`
}

func briefSectionStats(content, section string) briefSizeReport {
	report := briefSizeReport{BytesInBrief: len(content)}
	lines := strings.Split(content, "\n")
	headingAt := -1
	for i, l := range lines {
		if isBriefHeadingFor(l, section) {
			headingAt = i
			break
		}
	}
	if headingAt < 0 {
		// The honest answer for a heading appendToBrief would have to create.
		return report
	}
	end := len(lines)
	for i := headingAt + 1; i < len(lines); i++ {
		if isBriefHeading(lines[i]) {
			end = i
			break
		}
	}
	body := lines[headingAt+1 : end]
	report.BytesInSection = len(strings.Join(body, "\n"))
	for _, l := range body {
		if t := strings.TrimSpace(l); strings.HasPrefix(t, "- ") || strings.HasPrefix(t, "* ") {
			report.EntriesInSection++
		}
	}
	return report
}

// briefAppend appends one line to one section of the brief at `briefPath`,
// atomically. `briefPath` is already resolved and already confined by the
// caller — this function does no authorization, exactly like its twin.
func briefAppend(briefPath, section, line string) (map[string]any, error) {
	// Normalize (and refuse an empty or over-long line) BEFORE taking the lock:
	// a caller error should not make anyone else wait.
	bullet, err := normalizeBriefLine(line)
	if err != nil {
		return nil, err
	}
	if err := os.MkdirAll(filepath.Dir(briefPath), 0o755); err != nil {
		return nil, err
	}

	var result map[string]any
	lockErr := withFileLock(briefPath, briefLockWait, briefLockStale, func() error {
		var lastErr error
		for attempt := 0; attempt < briefCASAttempts; attempt++ {
			before, existed, err := readBriefOrEmpty(briefPath)
			if err != nil {
				return err
			}
			next := appendToBrief(before, section, bullet)
			// COMPARE-AND-SWAP: re-read immediately before publishing. An agent's
			// Edit tool does not take our lock, so "nobody changed it while I was
			// computing" has to be checked.
			check, _, err := readBriefOrEmpty(briefPath)
			if err != nil {
				return err
			}
			if check != before {
				lastErr = errors.New("brief.md changed under us")
				continue // recompute against the writer that beat us
			}
			if err := writeFileAtomic(briefPath, []byte(next), 0o644); err != nil {
				return err
			}
			stats := briefSectionStats(next, section)
			result = map[string]any{
				"path":    briefPath,
				"section": section,
				"line":    bullet,
				"created": !existed,
				// Measured on the bytes just published, not on a re-read: a
				// re-read would report whatever an outside writer did in between
				// as if this call had produced it.
				"entriesInSection": stats.EntriesInSection,
				"bytesInSection":   stats.BytesInSection,
				"bytesInBrief":     stats.BytesInBrief,
			}
			return nil
		}
		reason := "unknown"
		if lastErr != nil {
			reason = lastErr.Error()
		}
		return fmt.Errorf("brief.append: %s is being rewritten by another writer faster than this "+
			"could land a line (%d attempts). Nothing was written — retry, or edit the file directly. Last: %s",
			briefPath, briefCASAttempts, reason)
	})
	if lockErr != nil {
		return nil, lockErr
	}
	return result, nil
}

func readBriefOrEmpty(p string) (content string, exists bool, err error) {
	data, err := os.ReadFile(p)
	if errors.Is(err, os.ErrNotExist) {
		return "", false, nil
	}
	if err != nil {
		return "", false, err
	}
	return string(data), true, nil
}

// briefAppendCall is the capability. The guard runs FIRST and the brief path is
// composed under the guard's CANONICAL answer — see this file's header.
func (r *registry) briefAppendCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Project string `json:"project"`
		Section string `json:"section"`
		// Line is a pointer so "" (a deliberate empty line) is distinguishable
		// from the field being absent, matching the twin's `line === undefined`
		// check — the two get different refusals.
		Line *string `json:"line"`
		// The append-from-result params. Both optional and both pointers, so
		// "absent" is distinguishable from "empty" — an explicit sessionId of ""
		// is a caller error worth refusing, while an absent one just means a
		// line with no reference. With neither present this capability behaves
		// EXACTLY as it always has. See briefresult.go.
		SessionID *string        `json:"sessionId"`
		Result    map[string]any `json:"result"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if strings.TrimSpace(p.Project) == "" {
		return nil, fmt.Errorf("brief.append requires { project }")
	}
	if p.Line == nil {
		return nil, fmt.Errorf("brief.append requires { line }")
	}
	section, err := parseBriefSection(p.Section)
	if err != nil {
		return nil, err
	}
	roots := r.workspaceRoots(ctx)
	// The caller's only path input, held to the roots fs.write takes.
	dir, err := assertPathAllowed("brief.append", p.Project, roots)
	if err != nil {
		return nil, err
	}
	// AND THE COMPOSED PATH, held to the SAME roots. Guarding `project` alone is
	// not enough and this is not a theoretical gap — it was caught by
	// TestBriefAppendDoesNotFollowASymlinkedWorkspacerDirectory against the
	// first draft of this file, which did exactly what the desktop twin does.
	//
	// The hole: `project` can be a perfectly legitimate allowed directory — the
	// caller's own agent cwd — while `<project>/.workspacer` is a SYMLINK
	// pointing out of every root. The first guard resolves the directory it was
	// handed and says yes, truthfully; the components composed BELOW it were
	// never resolved by anything, so the write followed the link and landed
	// outside. "The caller cannot name a file" bounds the BASENAME and says
	// nothing about the directories on the way to it.
	//
	// So the operand is the canonical answer for the WHOLE path, and it is that
	// answer that gets opened — the same binding decision git.go's
	// anchorGitPathspec states, applied to a composed path rather than a
	// caller-supplied one. assertPathAllowed resolves per component and tolerates
	// a leaf that does not exist yet, which is what fs.write already relies on.
	briefPath, err := assertPathAllowed("brief.append", briefPathFor(dir), roots)
	if err != nil {
		return nil, err
	}
	// The composed line when the caller asked for one, the caller's own line
	// otherwise — byte for byte, so plain brief.append is untouched by this.
	text := *p.Line
	if p.SessionID != nil || p.Result != nil {
		sessionID := ""
		if p.SessionID != nil {
			sessionID = *p.SessionID
		}
		if text, err = composeResultLine(*p.Line, sessionID, p.Result, time.Now()); err != nil {
			return nil, err
		}
	}
	res, err := briefAppend(briefPath, section, text)
	if err != nil {
		return nil, err
	}
	return jsonResult(res)
}

// withFileLock is briefService's withFileLock: the O_EXCL advisory lock
// configlock.go takes, with per-caller wait and stale budgets.
//
// It is a separate function rather than a parameterization of withConfigLock
// because the STALE threshold is a correctness parameter shared with a
// cross-language twin for config.yaml (contracts/config-lock.json) and is NOT
// shared here — nothing else takes the brief lock. Folding the two would invite
// a future edit to "unify" the thresholds and silently change the config
// contract.
func withFileLock(path string, maxWait, staleAfter time.Duration, fn func() error) error {
	lockPath := path + lockFileSuffix
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	deadline := time.Now().Add(maxWait)
	for {
		f, err := os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o644)
		if err == nil {
			_, _ = fmt.Fprintf(f, "%d %s\n", os.Getpid(), time.Now().UTC().Format(time.RFC3339))
			_ = f.Close()
			break
		}
		if !os.IsExist(err) {
			return err
		}
		if st, statErr := os.Stat(lockPath); statErr == nil && time.Since(st.ModTime()) > staleAfter {
			_ = os.Remove(lockPath) // holder died mid-write; a lost race re-races below
			continue
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("brief.md is locked by another writer (waited %dms): %s", maxWait.Milliseconds(), lockPath)
		}
		time.Sleep(lockRetry)
	}
	defer func() { _ = os.Remove(lockPath) }()
	return fn()
}
