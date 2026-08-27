package main

// THE BRIEF'S DOCUMENT MODEL, headless — a `.workspacer/brief.md` read as
// ENTRIES rather than as lines, plus the one move that takes an entry out of it.
//
// TWIN: apps/desktop/src/main/shared/briefBoard.ts (parseBrief,
// removeEntryLines, appendToArchive, isPrependSection) and
// services/briefBoardService.ts (archiveOldestEntries). Held equal by
// contracts/brief-board-cases.json, which runs the same briefs through both
// copies — because "an entry" is a boundary rule, not a data structure, and two
// hand-ported boundary rules that disagree put the same bullet in two different
// places depending on which provider answered.
//
// WHY IT IS HERE NOW. brief.go's header used to record this model as the reason
// brief.check and brief.archive were desktop-only, and that reasoning has been
// overtaken: the node runs the MCP facade and carries synced briefs, so a
// headless manager is a real thing that reads its own brief with two of the
// three tools its doctrine names missing. See brief.go for the full note.
//
// THE THREE PROPERTIES CARRIED OVER FROM THE TWIN, unchanged:
//
//  1. ROUND-TRIP IS BYTE-EXACT. Nothing here reflows, normalizes or rewrites a
//     line. The document is its own []string plus INDEXES into it, so
//     strings.Join(doc.Lines, "\n") is the input byte for byte.
//  2. A MOVE ONLY MOVES. archiveOldestEntries splices WHOLE LINES out of one
//     file and appends them verbatim to another. The user's hand-written
//     wording is the authoritative thing in a brief and it comes out of a move
//     character for character.
//  3. THE ARCHIVE IS WRITTEN FIRST. A crash between the two writes must
//     DUPLICATE an entry, never lose one — the archive is append-only cold
//     storage where a duplicate is a nuisance, and the brief is the document a
//     manager's memory depends on.
//
// WHAT IS DELIBERATELY NOT PORTED: `entryId`, `deriveCard`, `moveEntryToColumn`
// and the sidecar index. Those serve the BOARD (a renderer surface with no
// headless caller), and the two capabilities this file exists for name a
// SECTION and a count, never an entry id. Porting the card derivation would be
// carrying a second copy of a hashing scheme that nothing on this side reads.

import (
	"context"
	"encoding/json"
	"fmt"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

// briefArchiveFileName is the third path component the caller never supplies —
// the sibling of briefFileName, composed by the provider under the SAME guarded
// directory. TWIN: ARCHIVE_FILE.
const briefArchiveFileName = "brief.archive.md"

// briefArchiveSkeleton is what an absent archive is created as.
// TWIN: ARCHIVE_SKELETON.
const briefArchiveSkeleton = "# Brief archive\n\nCold storage for entries pruned from the brief — never rewritten, only appended.\n"

// The twin's three regexps, with JavaScript's `\s` written out (jsWhitespace,
// see dispatchparams.go) rather than spelled `\s`.
//
// This is not fussiness. Go's regexp `\s` is [\t\n\f\r ] — no vertical tab, no
// NBSP — so a heading or a bullet padded with one of those parses as a heading
// on the desktop and as ordinary prose here, which moves an entry's BOUNDARY.
// A boundary that differs by one line is an archive move that takes a
// continuation line the other provider would have left, in the user's own
// document.
var (
	// ^(#{1,6})\s+(.*?)\s*$ — TWIN: HEADING_RE.
	briefBoardHeadingRe = regexp.MustCompile(`^(#{1,6})[` + jsWhitespace + `]+(.*?)[` + jsWhitespace + `]*$`)
	// ^#{1,6}\s+\S — TWIN: isHeading.
	briefBoardIsHeadingRe = regexp.MustCompile(`^#{1,6}[` + jsWhitespace + `]+[^` + jsWhitespace + `]`)
	// ^ {0,3}(?:[-*+]|\d+[.)])\s+\S — a top-level list bullet, with at most three
	// leading spaces (four would be an indented continuation, per CommonMark).
	// TWIN: BULLET_RE.
	briefBoardBulletRe = regexp.MustCompile(`^ {0,3}(?:[-*+]|\d+[.)])[` + jsWhitespace + `]+[^` + jsWhitespace + `]`)
)

// briefPrependSection reports whether a section is written newest-first, from
// the one place that rule is written down in this package (brief.go's
// briefPrependSections, which brief.append already inserts by).
// TWIN: isPrependSection.
func briefPrependSection(name string) bool {
	return briefPrependSections[strings.ToLower(trimJS(name))]
}

// briefBoardIsHeading reports whether a line ends the block above it.
func briefBoardIsHeading(line string) bool { return briefBoardIsHeadingRe.MatchString(line) }

// briefIsEntryStart reports whether a line OPENS an entry. TWIN: isEntryStart.
func briefIsEntryStart(line string) bool {
	return briefBoardBulletRe.MatchString(line) && !briefBoardHeadingRe.MatchString(line)
}

// briefSectionBlock is one `#{2,6}` heading and the body beneath it, as line
// indexes. A body ends at the next heading of ANY level — the same boundary
// brief.append uses, so the two insert and splice against the same block.
// TWIN: BriefSectionBlock.
type briefSectionBlock struct {
	Title       string
	Level       int
	HeadingLine int
	BodyStart   int
	BodyEnd     int
	// Column is the enclosing `##` section's title — itself when Level is 2. A
	// `###` sub-heading's entries belong to their parent section but splice
	// against their own block.
	Column string
}

// briefEntry is a top-level bullet and any continuation lines under it.
// TWIN: BriefEntry, minus `id` (see this file's header).
type briefEntry struct {
	Column string
	Group  string
	Start  int
	End    int
	Lines  []string
	Text   string
}

// briefDoc is a parsed brief. Lines joined with "\n" restores the input
// exactly, trailing newline or not, CRLF or not (a '\r' rides along at the end
// of its line). TWIN: BriefDoc.
type briefDoc struct {
	Lines    []string
	Sections []briefSectionBlock
	Entries  []briefEntry
}

// parseBriefDoc parses a brief into sections and entries WITHOUT consuming it.
// TWIN: parseBrief.
func parseBriefDoc(content string) briefDoc {
	lines := strings.Split(content, "\n")
	var sections []briefSectionBlock

	for i := 0; i < len(lines); i++ {
		m := briefBoardHeadingRe.FindStringSubmatch(lines[i])
		if m == nil || len(m[1]) < 2 {
			continue // the document's own `# Title` is not a section
		}
		end := len(lines)
		for j := i + 1; j < len(lines); j++ {
			if briefBoardIsHeading(lines[j]) {
				end = j
				break
			}
		}
		level := len(m[1])
		// The enclosing `##`. Walking backwards costs nothing at brief scale and
		// means a `####` under a `###` still resolves to the right section.
		column := m[2]
		if level > 2 {
			for k := len(sections) - 1; k >= 0; k-- {
				if sections[k].Level < level {
					column = sections[k].Column
					break
				}
			}
		}
		sections = append(sections, briefSectionBlock{
			Title: m[2], Level: level, HeadingLine: i, BodyStart: i + 1, BodyEnd: end, Column: column,
		})
	}

	var entries []briefEntry
	for _, sec := range sections {
		for i := sec.BodyStart; i < sec.BodyEnd; i++ {
			if !briefIsEntryStart(lines[i]) {
				continue
			}
			// Continuations: indented or plain prose lines that follow. A blank
			// line, a new bullet or a heading closes the entry — so a blank
			// separator the author put between entries stays with the SECTION
			// and never travels with the entry.
			end := i + 1
			for end < sec.BodyEnd &&
				trimJS(lines[end]) != "" &&
				!briefIsEntryStart(lines[end]) &&
				!briefBoardIsHeading(lines[end]) {
				end++
			}
			slice := append([]string(nil), lines[i:end]...)
			group := ""
			if sec.Level > 2 {
				group = sec.Title
			}
			entries = append(entries, briefEntry{
				Column: sec.Column, Group: group, Start: i, End: end,
				Lines: slice, Text: strings.Join(slice, "\n"),
			})
			i = end - 1
		}
	}

	return briefDoc{Lines: lines, Sections: sections, Entries: entries}
}

// removeBriefEntryLines removes an entry's lines. Returns the new line slice;
// the caller re-parses. TWIN: removeEntryLines.
func removeBriefEntryLines(lines []string, entry briefEntry) []string {
	next := make([]string, 0, len(lines)-(entry.End-entry.Start))
	next = append(next, lines[:entry.Start]...)
	next = append(next, lines[entry.End:]...)
	return next
}

// appendToBriefArchive appends an entry's lines to `brief.archive.md` under a
// `## <date>` batch heading, creating the file or the heading as needed.
//
// APPEND-ONLY, per the /checkpoint doctrine: an existing line in the archive is
// never touched, so a batch heading that is already there gets the entry added
// at its end rather than the file being reorganized around it.
// TWIN: appendToArchive.
func appendToBriefArchive(archive string, entryLines []string, date string) string {
	base := archive
	if trimJS(archive) == "" {
		base = briefArchiveSkeleton
	}
	lines := strings.Split(base, "\n")

	headingAt := -1
	for i := range lines {
		m := briefBoardHeadingRe.FindStringSubmatch(lines[i])
		if m != nil && len(m[1]) == 2 && trimJS(m[2]) == date {
			headingAt = i
			break
		}
	}

	if headingAt < 0 {
		trailing := "\n"
		if lines[len(lines)-1] == "" {
			trailing = ""
		}
		return base + trailing + "\n## " + date + "\n" + strings.Join(entryLines, "\n") + "\n"
	}

	end := len(lines)
	for i := headingAt + 1; i < len(lines); i++ {
		if briefBoardIsHeading(lines[i]) {
			end = i
			break
		}
	}
	at := end
	for at > headingAt+1 && trimJS(lines[at-1]) == "" {
		at--
	}
	next := make([]string, 0, len(lines)+len(entryLines))
	next = append(next, lines[:at]...)
	next = append(next, entryLines...)
	next = append(next, lines[at:]...)
	return strings.Join(next, "\n")
}

// briefArchivePathFor resolves a project directory to its archive path.
func briefArchivePathFor(projectDir string) string {
	return filepath.Join(projectDir, briefDirName, briefArchiveFileName)
}

// briefTodayStamp is the archive's batch heading: LOCAL YYYY-MM-DD. Local and
// not UTC for the twin's reason — the archive's headings are the user's dates,
// and a UTC one would file an evening move under tomorrow. TWIN: todayStamp.
func briefTodayStamp(now time.Time) string { return now.Format("2006-01-02") }

// briefArchiveResult is what brief.archive answers with. The three size fields
// are briefSizeReport's, flattened exactly as the twin spreads BriefSizeReport
// into BriefArchiveResult.
type briefArchiveResult struct {
	Path        string `json:"path"`
	ArchivePath string `json:"archivePath"`
	Section     string `json:"section"`
	// Archived is how many entries actually moved.
	Archived int `json:"archived"`
	// Date is the `## <date>` heading they landed under.
	Date             string `json:"date"`
	EntriesInSection int    `json:"entriesInSection"`
	BytesInSection   int    `json:"bytesInSection"`
	BytesInBrief     int    `json:"bytesInBrief"`
}

// withBriefCAS runs `mutate` against the brief under the SAME lock and the SAME
// compare-and-swap brief.append takes.
//
// `mutate` returns the brief's new content plus an optional side effect to
// perform ONCE THE CAS HAS PASSED — which is where the archive write goes. A
// side effect performed inside the compute would be repeated by every retry,
// and for the archive that means a duplicated entry per attempt.
// TWIN: briefBoardService's withBrief.
func withBriefCAS(briefPath string, mutate func(content string) (string, func() error, error)) error {
	return withFileLock(briefPath, briefLockWait, briefLockStale, func() error {
		for attempt := 0; attempt < briefCASAttempts; attempt++ {
			before, existed, err := readBriefOrEmpty(briefPath)
			if err != nil {
				return err
			}
			// The seam the CAS is only observable through: an outside write has
			// to land BETWEEN this read and the re-read below. A no-op in
			// production; tests override it to be the agent Edit tool that does
			// not take this lock. Same shape and same reason as config.go's
			// preWriteHook, and the same technique the desktop twin's tests use
			// (they divert fs.readFileSync). See
			// TestBriefArchiveRetriesAgainstAnOutsideWriterWithoutDuplicating.
			briefCASHook(briefPath)
			if !existed {
				// A missing brief is not "an empty brief" here, unlike
				// brief.append: there is nothing to trim, and creating one to
				// take entries out of would be absurd. TWIN: `no brief at`.
				return fmt.Errorf("brief board: no brief at %s", briefPath)
			}
			next, beforeWrite, err := mutate(before)
			if err != nil {
				return err
			}
			// Re-read immediately before publishing: an agent's Edit tool and
			// the user's editor do not take this lock, so "nothing changed
			// while I was computing" is a claim to check, not to assume.
			check, _, err := readBriefOrEmpty(briefPath)
			if err != nil {
				return err
			}
			if check != before {
				continue // recompute against the writer that beat us
			}
			if next == before {
				return nil
			}
			if beforeWrite != nil {
				if err := beforeWrite(); err != nil {
					return err
				}
			}
			return writeFileAtomic(briefPath, []byte(next), 0o644)
		}
		return fmt.Errorf("brief board: %s is being rewritten faster than this move could land "+
			"(%d attempts). Nothing was written — reload the board and try again.", briefPath, briefCASAttempts)
	})
}

// briefCASHook is the test seam withBriefCAS calls once per attempt. See its
// call site.
var briefCASHook = func(string) {}

// archiveOldestEntries moves the OLDEST entries of one section out to
// `brief.archive.md`.
//
// WHICH END IS THE OLDEST. `## Recently` is kept newest-first, so its oldest
// entries are its LAST; every other section is written in arrival order, so its
// oldest are its FIRST. That is briefPrependSection, the same rule brief.append
// inserts by, read from the one place it is written down.
//
// `dir` is ALREADY confined by the caller — this function does no
// authorization, exactly like its twin, and it composes both basenames itself.
// TWIN: archiveOldestEntries.
func archiveOldestEntries(dir, section string, count, keep *int, date string) (briefArchiveResult, error) {
	briefPath := briefPathFor(dir)
	archivePath := briefArchivePathFor(dir)
	wanted := strings.ToLower(trimJS(section))

	// Exactly one of the two, because they answer different questions and a
	// caller that gave both has not decided which. `keep` is the idempotent form
	// (run it twice, the second run moves nothing), which is why /checkpoint
	// uses it; `count` is for a caller that has already counted.
	if (count == nil) == (keep == nil) {
		return briefArchiveResult{}, fmt.Errorf(
			"brief.archive: give either count (archive this many of the oldest) or keep " +
				"(leave this many of the newest), and not both")
	}
	bound := count
	which := "count"
	if bound == nil {
		bound, which = keep, "keep"
	}
	if *bound < 0 {
		return briefArchiveResult{}, fmt.Errorf("brief.archive: %s must be a whole number, 0 or more", which)
	}

	archived := 0
	err := withBriefCAS(briefPath, func(content string) (string, func() error, error) {
		doc := parseBriefDoc(content)
		hasSection := false
		for _, s := range doc.Sections {
			if s.Level == 2 && strings.ToLower(s.Title) == wanted {
				hasSection = true
				break
			}
		}
		if !hasSection {
			// TWIN: BriefColumnMissing.
			return "", nil, fmt.Errorf("brief board: this brief has no %q section", "## "+section)
		}

		var inSection []briefEntry
		for _, e := range doc.Entries {
			if strings.ToLower(e.Column) == wanted {
				inSection = append(inSection, e)
			}
		}
		oldestFirst := inSection
		if briefPrependSection(section) {
			oldestFirst = make([]briefEntry, 0, len(inSection))
			for i := len(inSection) - 1; i >= 0; i-- {
				oldestFirst = append(oldestFirst, inSection[i])
			}
		}
		want := *bound
		if count == nil {
			want = max(0, len(inSection)-*keep)
		}
		if want > len(inSection) {
			want = len(inSection)
		}
		chosen := map[int]bool{}
		for _, e := range oldestFirst[:want] {
			chosen[e.Start] = true
		}
		// Back in document order, so the archive reads the way the section did.
		var moving []briefEntry
		for _, e := range inSection {
			if chosen[e.Start] {
				moving = append(moving, e)
			}
		}
		archived = len(moving)

		// Splice from the bottom up: every entry's line indexes point into the
		// same slice, and removing an earlier one would shift the rest.
		lines := doc.Lines
		for i := len(moving) - 1; i >= 0; i-- {
			lines = removeBriefEntryLines(lines, moving[i])
		}

		return strings.Join(lines, "\n"), func() error {
			// ARCHIVE FIRST, and only once the compare-and-swap has passed —
			// see this file's header. The archive is read HERE rather than
			// outside, so a retry composes against what is on disk now. (It is
			// not separately locked: it is only ever appended to, and this
			// brief's lock serializes every writer that touches it.)
			next, _, err := readBriefOrEmpty(archivePath)
			if err != nil {
				return err
			}
			for _, e := range moving {
				next = appendToBriefArchive(next, e.Lines, date)
			}
			return writeFileAtomic(archivePath, []byte(next), 0o644)
		}, nil
	})
	if err != nil {
		return briefArchiveResult{}, err
	}

	after, _, err := readBriefOrEmpty(briefPath)
	if err != nil {
		return briefArchiveResult{}, err
	}
	stats := briefSectionStats(after, section)
	return briefArchiveResult{
		Path: briefPath, ArchivePath: archivePath, Section: section,
		Archived: archived, Date: date,
		EntriesInSection: stats.EntriesInSection,
		BytesInSection:   stats.BytesInSection,
		BytesInBrief:     stats.BytesInBrief,
	}, nil
}

// briefArchiveCall is the capability. The guard runs FIRST and BOTH composed
// paths are asserted under the guard's canonical answer, for the reason
// brief.append's own comment spells out: `project` can be a legitimate allowed
// directory while `<project>/.workspacer` is a symlink pointing out of every
// root. This verb moves entries BETWEEN the two files, so an unguarded compose
// could both read a brief and write an archive outside every root.
func (r *registry) briefArchiveCall(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Project string `json:"project"`
		Section string `json:"section"`
		// Decoded as float64 rather than int so a fractional 1.5 is a CALLER
		// ERROR with the twin's message rather than a silent truncation to 1.
		// TWIN: Number.isInteger.
		Count *float64 `json:"count"`
		Keep  *float64 `json:"keep"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if strings.TrimSpace(p.Project) == "" {
		return nil, fmt.Errorf("brief.archive requires { project }")
	}
	section, err := parseBriefSection(p.Section)
	if err != nil {
		return nil, err
	}
	count, err := wholeBriefBound("count", p.Count)
	if err != nil {
		return nil, err
	}
	keep, err := wholeBriefBound("keep", p.Keep)
	if err != nil {
		return nil, err
	}
	roots := r.workspaceRoots(ctx)
	dir, err := assertPathAllowed("brief.archive", p.Project, roots)
	if err != nil {
		return nil, err
	}
	// The guarded answers are discarded rather than threaded through
	// archiveOldestEntries — which takes the directory and composes both
	// basenames itself — because the assertion is the point: it throws on an
	// escape, and past it the directory really does contain both files.
	if _, err := assertPathAllowed("brief.archive", briefPathFor(dir), roots); err != nil {
		return nil, err
	}
	if _, err := assertPathAllowed("brief.archive", briefArchivePathFor(dir), roots); err != nil {
		return nil, err
	}
	res, err := archiveOldestEntries(dir, section, count, keep, briefTodayStamp(time.Now()))
	if err != nil {
		return nil, err
	}
	return jsonResult(res)
}

// wholeBriefBound turns a JSON number into the whole count the mover takes, or
// refuses it. TWIN: the Number.isInteger check in archiveOldestEntries.
func wholeBriefBound(which string, v *float64) (*int, error) {
	if v == nil {
		return nil, nil
	}
	if *v != float64(int(*v)) || *v < 0 {
		return nil, fmt.Errorf("brief.archive: %s must be a whole number, 0 or more", which)
	}
	n := int(*v)
	return &n, nil
}
