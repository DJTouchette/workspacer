package main

// READ-ONLY git for the headless brain: git.status / git.log / git.numstat /
// git.diff.
//
// TWIN: apps/desktop/src/main/services/gitService.ts (the implementation) and
// the git block of apps/desktop/src/main/services/hubCapabilities.ts (the
// confinement). The desktop registers those methods inside Electron main, so a
// node running `brain --hub <ws>` answered every one of them with "no provider"
// — the branch chip, the Review pane, the rail git widget, the per-turn
// changed-file counts and the MCP `project_status` tool all went blank on a
// remote worker. Same methods, same wire shapes, second provider.
//
// WHAT IS DELIBERATELY ABSENT: git.stage, git.unstage, git.commit, git.push
// (and git.commitDiff / git.commitNumstat, which are read-only but were not
// asked for). This provider runs on an internet-facing box; a read-only surface
// cannot mutate or publish a repository, and a bus token that reaches this
// daemon must not be able to commit from infrastructure the user is not sitting
// at. Those four stay declared gaps in headless_completeness_test.go so the
// Review pane's buttons fail with a real message rather than silently.
//
// The confinement is the point of the port, not a decoration on it. Every entry
// point takes a caller-supplied `cwd`, and `git.diff` additionally takes a
// caller-supplied `path` that git resolves as a FILESYSTEM operand on the
// untracked leg. Both are held by fsguard.go's assertPathAllowed — see
// guardGitCwd and anchorGitPathspec below, which carry the two holes the
// desktop twin already had to close.

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
)

// gitMaxOutput caps a single git command's stdout. Mirrors the desktop's
// MAX_BUFFER (256 MB): a whole-work-tree diff can be large, and the renderer
// gates RENDERING past ~1.5 MB while still receiving the full text, so the cap
// has to leave headroom before truncation would corrupt a diff. Exceeding it is
// an error, not a truncation — half a diff is worse than no diff, and Node's
// maxBuffer kills the child the same way.
const gitMaxOutput = 256 * 1024 * 1024

// gitDiffFamily are the subcommands that generate a diff and therefore honour
// `diff.external` and the per-driver textconv/command drivers.
//
// TWIN: DIFF_FAMILY in apps/desktop/src/main/lib/gitExec.ts, pinned by
// TestGitDiffFamilyMatchesTheDesktopTwin.
var gitDiffFamily = map[string]bool{
	"diff":         true,
	"show":         true,
	"log":          true,
	"format-patch": true,
	"diff-index":   true,
	"diff-tree":    true,
	"diff-files":   true,
	"range-diff":   true,
	"whatchanged":  true,
}

// gitArgs is `git` argv with the no-exec prefix in front of the subcommand and
// `--no-ext-diff` immediately after it when the subcommand generates a diff.
//
// gitNoExecConfig() (fsops.go) neutralizes every exec-valued config key with a
// FIXED name. `diff.external` is not one of them and CANNOT be: git treats an
// empty external diff as a command to run and dies with "cannot run :", so the
// `-c` spelling breaks every real diff. `--no-ext-diff` is git's own documented
// off switch, and it is a diff OPTION rather than a global one — which is why it
// has to be inserted after the subcommand here rather than living in the prefix.
// Until this file existed the daemon only ever ran `check-ignore`, which has no
// diff; `git diff` on a caller-writable work tree does, so this is the leg the
// brain was missing.
//
// The subcommand is the first element that is not a `-c` pair, because callers
// pass their own leading `-c` (gitNumstat passes core.quotepath=false).
//
// TWIN: gitArgs in apps/desktop/src/main/lib/gitExec.ts.
func gitArgs(argv []string) []string {
	prefix := gitNoExecConfig()
	out := make([]string, 0, len(prefix)+len(argv)+1)
	out = append(out, prefix...)
	out = append(out, argv...)
	for i := len(prefix); i < len(out); i++ {
		if out[i] == "-c" {
			i++ // skip the key=value that follows
			continue
		}
		if gitDiffFamily[out[i]] {
			rest := append([]string{"--no-ext-diff"}, out[i+1:]...)
			out = append(out[:i+1], rest...)
		}
		break
	}
	return out
}

// errGitOutputTooLarge is the cap being hit, kept distinct so the message a
// caller sees names the cap rather than a broken pipe.
var errGitOutputTooLarge = errors.New("git output exceeded the size cap")

// cappedWriter refuses past `limit` bytes, which breaks git's stdout pipe and
// ends the command instead of letting the daemon buffer an unbounded diff.
type cappedWriter struct {
	buf   bytes.Buffer
	limit int
	over  bool
}

func (c *cappedWriter) Write(p []byte) (int, error) {
	if c.buf.Len()+len(p) > c.limit {
		c.over = true
		return 0, errGitOutputTooLarge
	}
	return c.buf.Write(p)
}

// gitResult is one git invocation's outcome. `ok` is git's own exit status:
// runGit never fails a call on a non-zero exit, because callers decide what a
// failure means (a read fails the request; `git log` in an empty repo is
// legitimately non-zero and yields no commits).
type gitResult struct {
	ok     bool
	stdout string
	stderr string
}

// runGit runs `git` in `dir` with the no-exec prefix. It errors only when git
// could not be RUN — a missing binary, or output past the cap.
//
// `dir` is always a path assertPathAllowed returned (BINDING DECISION 2): the
// directory that was checked and the directory git runs in have to be the same
// string, or a symlinked cwd is validated in one place and used in another.
func runGit(ctx context.Context, dir string, argv []string) (gitResult, error) {
	args := gitArgs(argv)
	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Dir = dir
	stdout := &cappedWriter{limit: gitMaxOutput}
	stderr := &cappedWriter{limit: 1 << 20}
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	err := cmd.Run()
	if stdout.over {
		return gitResult{}, fmt.Errorf("%w (%d bytes)", errGitOutputTooLarge, gitMaxOutput)
	}
	if err != nil {
		// git not on PATH — distinct from a non-zero git exit, and the one
		// failure worth naming: a box with no git binary otherwise produces a
		// mystery instead of an instruction.
		if errors.Is(err, exec.ErrNotFound) {
			return gitResult{}, errors.New("could not run git (is it installed and on PATH?)")
		}
		var exitErr *exec.ExitError
		if !errors.As(err, &exitErr) {
			return gitResult{}, err
		}
	}
	return gitResult{ok: err == nil, stdout: stdout.buf.String(), stderr: stderr.buf.String()}, nil
}

// gitWorkRoot resolves `cwd` to its git work-tree root, or "" when `cwd` is not
// inside one.
//
// Every command below runs from this root rather than `cwd` itself, because
// `git status` / `diff --numstat` emit REPO-ROOT-RELATIVE paths while
// `git diff` interprets a pathspec relative to the CURRENT DIRECTORY. Run from a
// subdirectory those two conventions disagree, so a root-relative path silently
// matches nothing and the diff comes back empty. Anchoring at the root keeps
// both ends speaking the same path language.
//
// The root is DERIVED — it comes out of git, after the cwd guard, and nothing
// ever checked it against the allow-list. anchorGitPathspec is where that
// matters; see its comment.
func gitWorkRoot(ctx context.Context, cwd string) (string, error) {
	res, err := runGit(ctx, cwd, []string{"rev-parse", "--show-toplevel"})
	if err != nil {
		return "", err
	}
	if !res.ok {
		return "", nil
	}
	return strings.TrimSpace(res.stdout), nil
}

// gitRootOrErr is gitWorkRoot with the daemon's own refusal for "not a repo".
func gitRootOrErr(ctx context.Context, cwd string) (string, error) {
	root, err := gitWorkRoot(ctx, cwd)
	if err != nil {
		return "", err
	}
	if root == "" {
		return "", errors.New("cwd is not inside a git work tree")
	}
	return root, nil
}

// ── wire shapes (TWIN: gitService.ts FileStatus/GitStatus/NumstatEntry/LogEntry
//    and ipcTypes.ts, which the web client's GitClient decodes) ───────────────

// gitFileStatus is one changed file from `git status --porcelain`. `Staged` and
// `Unstaged` are the porcelain XY codes ("M", "A", "D", "?", " ").
type gitFileStatus struct {
	Path string `json:"path"`
	// OrigPath is set only for renames/copies: the original path. Omitted
	// rather than null, matching the desktop's optional `orig_path`.
	OrigPath string `json:"orig_path,omitempty"`
	Staged   string `json:"staged"`
	Unstaged string `json:"unstaged"`
}

type gitStatusResult struct {
	Branch *string         `json:"branch"`
	Files  []gitFileStatus `json:"files"`
	// Upstream is the tracking branch ("origin/master"), or null when none is
	// configured (or it is gone).
	Upstream *string `json:"upstream"`
	Ahead    int     `json:"ahead"`
	Behind   int     `json:"behind"`
}

// gitNumstatEntry is one row of `git diff --numstat`. Null counts mean a binary
// file (numstat prints `-` for those).
type gitNumstatEntry struct {
	Path    string `json:"path"`
	Added   *int   `json:"added"`
	Deleted *int   `json:"deleted"`
}

type gitLogEntry struct {
	Hash    string `json:"hash"`
	Subject string `json:"subject"`
	// AuthoredAt is author time, unix seconds.
	AuthoredAt int64 `json:"authoredAt"`
}

// ── parsers ─────────────────────────────────────────────────────────────────

// parseGitPorcelain parses `git status --porcelain -z` into structured rows.
//
// Each entry is `XY <path>` terminated by NUL, where X is the staged (index)
// status and Y the unstaged (work tree) status. The `-z` format never quotes or
// escapes paths (unlike the default, which wraps unusual paths in "…"). For a
// rename/copy the destination path is this entry and the original path is the
// NEXT NUL-terminated token — a different format from the inline `old => new`
// numstat spelling below, which is why a fix in one does not fix the other.
func parseGitPorcelain(stdout string) []gitFileStatus {
	files := []gitFileStatus{}
	tokens := strings.Split(stdout, "\x00")
	for i := 0; i < len(tokens); i++ {
		entry := tokens[i]
		// Need at least "XY <path>" — two status chars, a space, then a path.
		if len(entry) < 4 {
			continue
		}
		staged := entry[0:1]
		unstaged := entry[1:2]
		path := entry[3:]

		// Rename/copy: the source path follows as a separate token (no " -> ").
		var orig string
		if staged == "R" || staged == "C" || unstaged == "R" || unstaged == "C" {
			i++
			if i < len(tokens) {
				orig = tokens[i]
			}
		}
		files = append(files, gitFileStatus{Path: path, OrigPath: orig, Staged: staged, Unstaged: unstaged})
	}
	return files
}

// parseGitNumstatPath resolves a numstat path to the NEW name. Renames appear
// either as `old => new` or in brace form `prefix/{old => new}/suffix`.
func parseGitNumstatPath(raw string) string {
	open := strings.Index(raw, "{")
	close := strings.Index(raw, "}")
	if open != -1 && close != -1 && open < close {
		inner := raw[open+1 : close]
		if arrow := strings.Index(inner, " => "); arrow != -1 {
			next := inner[arrow+4:]
			joined := raw[:open] + next + raw[close+1:]
			// An empty side ("{ => sub}") leaves a doubled separator behind.
			return strings.Replace(joined, "//", "/", 1)
		}
	}
	if arrow := strings.Index(raw, " => "); arrow != -1 {
		return raw[arrow+4:]
	}
	return raw
}

func parseGitNumstat(stdout string) []gitNumstatEntry {
	out := []gitNumstatEntry{}
	for _, line := range strings.Split(stdout, "\n") {
		if line == "" {
			continue
		}
		parts := strings.Split(line, "\t")
		if len(parts) < 3 {
			continue
		}
		rawPath := strings.TrimSuffix(strings.Join(parts[2:], "\t"), "\r")
		out = append(out, gitNumstatEntry{
			Path:    parseGitNumstatPath(rawPath),
			Added:   parseGitCount(parts[0]),
			Deleted: parseGitCount(parts[1]),
		})
	}
	return out
}

// parseGitCount turns a numstat count into a pointer, so numstat's "-" (binary)
// becomes JSON null exactly as the desktop's NaN → null does.
func parseGitCount(s string) *int {
	n, err := strconv.Atoi(strings.TrimSpace(s))
	if err != nil {
		return nil
	}
	return &n
}

var (
	gitAheadRe  = regexp.MustCompile(`ahead (\d+)`)
	gitBehindRe = regexp.MustCompile(`behind (\d+)`)
)

// parseGitBranchHeader parses the `--branch` header of porcelain status
// (`## master...origin/master [ahead 1, behind 2]`). Variants: no upstream
// (`## master`), a gone upstream (`[gone]` — treated as none, plain `git push`
// cannot reach it), detached (`## HEAD (no branch)`), and an unborn branch
// (`## No commits yet on x`).
func parseGitBranchHeader(header string) (upstream *string, ahead, behind int) {
	if !strings.HasPrefix(header, "## ") {
		return nil, 0, 0
	}
	body := header[3:]
	sep := strings.Index(body, "...")
	if sep == -1 {
		return nil, 0, 0
	}
	rest := body[sep+3:]
	if bracket := strings.Index(rest, " ["); bracket != -1 {
		inside := strings.TrimSuffix(rest[bracket+2:], "]")
		rest = rest[:bracket]
		if inside == "gone" {
			return nil, 0, 0
		}
		if m := gitAheadRe.FindStringSubmatch(inside); m != nil {
			ahead, _ = strconv.Atoi(m[1])
		}
		if m := gitBehindRe.FindStringSubmatch(inside); m != nil {
			behind, _ = strconv.Atoi(m[1])
		}
	}
	if rest == "" {
		return nil, ahead, behind
	}
	return &rest, ahead, behind
}

// parseGitLog parses `git log --pretty=format:%h%x00%s%x00%at` (one commit per
// line, NUL-separated fields — subjects never contain newlines or NULs).
func parseGitLog(stdout string) []gitLogEntry {
	out := []gitLogEntry{}
	for _, line := range strings.Split(stdout, "\n") {
		if line == "" {
			continue
		}
		fields := strings.Split(line, "\x00")
		if len(fields) < 3 || fields[0] == "" {
			continue
		}
		at, err := strconv.ParseInt(fields[2], 10, 64)
		if err != nil {
			continue
		}
		out = append(out, gitLogEntry{Hash: fields[0], Subject: fields[1], AuthoredAt: at})
	}
	return out
}

// ── operations (all read-only) ──────────────────────────────────────────────

func gitStatus(ctx context.Context, cwd string) (*gitStatusResult, error) {
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return nil, err
	}
	// `--untracked-files=all` lists every untracked file individually. Without
	// it git collapses a fully-untracked directory into one `dir/` entry, and
	// the review pane would then ask for an untracked diff of a directory.
	// `--branch` prepends a `## …` header carrying upstream + ahead/behind,
	// which the review pane uses to grey out Push when there is nothing to push.
	res, err := runGit(ctx, root, []string{"status", "--porcelain", "-z", "--branch", "--untracked-files=all"})
	if err != nil {
		return nil, err
	}
	if !res.ok {
		return nil, gitReadError(res, "git status failed")
	}

	// Split the branch header off before the file parser sees it.
	nul := strings.Index(res.stdout, "\x00")
	header := ""
	body := res.stdout
	if strings.HasPrefix(res.stdout, "## ") {
		if nul == -1 {
			header, body = res.stdout, ""
		} else {
			header, body = res.stdout[:nul], res.stdout[nul+1:]
		}
	}
	upstream, ahead, behind := parseGitBranchHeader(header)

	// Branch name is best-effort: a detached HEAD or a fresh repo may not have
	// one.
	var branch *string
	if b, err := runGit(ctx, root, []string{"rev-parse", "--abbrev-ref", "HEAD"}); err == nil && b.ok {
		if name := strings.TrimSpace(b.stdout); name != "" && name != "HEAD" {
			branch = &name
		}
	}

	return &gitStatusResult{
		Branch:   branch,
		Files:    parseGitPorcelain(body),
		Upstream: upstream,
		Ahead:    ahead,
		Behind:   behind,
	}, nil
}

// gitLog returns the most recent commits, newest first. An empty repo — where
// `git log` exits non-zero because HEAD has no commits — yields none.
func gitLog(ctx context.Context, cwd string, limit int) ([]gitLogEntry, error) {
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return nil, err
	}
	if limit <= 0 {
		limit = 5
	}
	if limit > 50 {
		limit = 50
	}
	res, err := runGit(ctx, root, []string{"log", "-n", strconv.Itoa(limit), "--pretty=format:%h%x00%s%x00%at"})
	if err != nil {
		return nil, err
	}
	if !res.ok {
		return []gitLogEntry{}, nil
	}
	return parseGitLog(res.stdout), nil
}

// gitNumstat returns added/deleted line counts per changed file.
func gitNumstat(ctx context.Context, cwd string, staged bool) ([]gitNumstatEntry, error) {
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return nil, err
	}
	// `core.quotepath=false` keeps unicode paths unquoted so they match the
	// (NUL-unquoted) paths from `git status`.
	args := []string{"-c", "core.quotepath=false", "diff", "--numstat"}
	if staged {
		args = append(args, "--staged")
	}
	res, err := runGit(ctx, root, args)
	if err != nil {
		return nil, err
	}
	if !res.ok {
		return nil, gitReadError(res, "git diff --numstat failed")
	}
	return parseGitNumstat(res.stdout), nil
}

// gitDiff returns unified diff text for a single file (or the whole work tree
// when `path` is empty). `staged` selects index-vs-HEAD; `untracked` renders an
// untracked file as an all-added diff via `--no-index`.
//
// `path` MUST already be the operand anchorGitPathspec returned — see the
// git.diff handler. This function does not confine anything; it is the twin of
// gitService.diff, and the guard lives at the capability the way it does on the
// desktop.
func gitDiff(ctx context.Context, cwd, path string, staged, untracked bool) (string, error) {
	root, err := gitRootOrErr(ctx, cwd)
	if err != nil {
		return "", err
	}

	if untracked {
		if path == "" {
			return "", errors.New("untracked diff requires a path")
		}
		// A directory has no single-file diff: `git diff --no-index /dev/null
		// dir/` makes git hunt for `dir/null` and fail. Status uses
		// --untracked-files=all so this should not happen, but guard rather than
		// emit a confusing error.
		if strings.HasSuffix(path, "/") || strings.HasSuffix(path, `\`) {
			return "", errors.New("untracked diff path is a directory")
		}
		// `--no-index` exits 1 when the files differ — the expected case here —
		// so success is "produced output", not "exit 0". git special-cases the
		// literal "/dev/null" on every platform, including Windows.
		res, err := runGit(ctx, root, []string{"diff", "--no-index", "--", "/dev/null", path})
		if err != nil {
			return "", err
		}
		if res.ok || res.stdout != "" {
			return res.stdout, nil
		}
		return "", gitReadError(res, "git diff failed")
	}

	args := []string{"diff"}
	if staged {
		args = append(args, "--staged")
	}
	// `--` separates pathspecs from revisions so a file named like a flag
	// cannot be misread as one.
	if path != "" {
		args = append(args, "--", path)
	}
	res, err := runGit(ctx, root, args)
	if err != nil {
		return "", err
	}
	if !res.ok {
		return "", gitReadError(res, "git diff failed")
	}
	return res.stdout, nil
}

// gitReadError is the reads' failure shape: git's own stderr verbatim, or the
// command's name when git said nothing. Matches gitService.ts, whose reads throw
// `res.stderr.trim() || '<cmd> failed'`.
func gitReadError(res gitResult, fallback string) error {
	if msg := strings.TrimSpace(res.stderr); msg != "" {
		return errors.New(msg)
	}
	return errors.New(fallback)
}

// ── confinement ─────────────────────────────────────────────────────────────

// guardGitCwd confines a caller-supplied `cwd` to the workspace roots and
// returns the CANONICAL directory git is then run in.
//
// TWIN: guardGitCwd in hubCapabilities.ts. Every git.* capability takes a
// caller-supplied cwd; without confinement a bus/token client could read the
// diff of any git repo this daemon's user can reach, and a symlinked cwd could
// point outside the intended repo. Canonicalization resolves symlinks BEFORE the
// check, and the canonical answer is what runGit is handed, so the directory
// that was checked and the directory git runs in are one string (BINDING
// DECISION 2).
func (r *registry) guardGitCwd(ctx context.Context, capability, cwd string) (string, error) {
	return assertPathAllowed(capability, cwd, r.workspaceRoots(ctx))
}

// anchorGitPathspec anchors a caller-supplied pathspec on the work-tree root git
// will actually resolve it in, holds the result to every root set in
// `extraRootSets`, and returns it in the root-relative form git wants.
//
// TWIN: anchorGitPathspec in hubCapabilities.ts, and this is the hole the port
// exists to carry rather than transliterate.
//
// THE DERIVED WORK-TREE ROOT is the thing to be careful about. Every command
// runs from `rev-parse --show-toplevel`, and that directory comes out of git
// AFTER the cwd guard — nothing ever checked it against the allow-list. So
// resolving a pathspec against the caller's `cwd` would check a different file
// than git opens whenever the agent cwd is a subdirectory (the ordinary monorepo
// case), and treating the root as trusted turns "a pathspec inside the confined
// repo" into "any path inside a repository that merely CONTAINS an allowed
// directory".
//
// CONCATENATION, NOT filepath.Join: Join Cleans, and Clean collapses a
// `link/..` pair textually before any symlink is read — precisely the
// check-path / opened-path split the component walk in canonicalizePath exists
// to close. assertPathAllowed does the resolving.
func (r *registry) anchorGitPathspec(ctx context.Context, capability, canonicalCwd, filePath string, extraRootSets [][]string) (string, error) {
	root, err := gitWorkRoot(ctx, canonicalCwd)
	if err != nil {
		return "", err
	}
	if root == "" {
		root = canonicalCwd
	}
	anchored := filePath
	if !filepath.IsAbs(filePath) {
		if strings.HasSuffix(root, string(filepath.Separator)) {
			anchored = root + filePath
		} else {
			anchored = root + string(filepath.Separator) + filePath
		}
	}
	// Always: inside the repository git is about to resolve the pathspec in.
	canonicalFile, err := assertPathAllowed(capability, anchored, []string{root})
	if err != nil {
		return "", err
	}
	// …plus whatever narrower boundary the particular leg demands. Each set is a
	// separate assertion, so a caller has to satisfy ALL of them.
	for _, roots := range extraRootSets {
		if _, err := assertPathAllowed(capability, anchored, roots); err != nil {
			return "", err
		}
	}
	// git runs from the work-tree root, so it receives the validated path
	// expressed from that root: the operand is a function of the CANONICAL path,
	// never of the caller's string. (Root-relative is what git wants for a
	// pathspec; the absolute form is the fallback for the degenerate "the path
	// IS the root" case, and for a Windows cross-volume root.)
	canonicalRootPath, ok := canonicalRoot(root)
	if !ok {
		canonicalRootPath = root
	}
	rel, err := filepath.Rel(canonicalRootPath, canonicalFile)
	if err != nil || rel == "." || rel == "" {
		return canonicalFile, nil
	}
	// Unreachable while assertPathAllowed's containment holds — canonicalFile
	// was just proven at-or-inside this root — and asserted rather than assumed,
	// because a `..` operand is a pathspec pointing OUT of the repository, which
	// is the entire escape this helper closes.
	if rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return "", fmt.Errorf("%s: path is outside the allowed workspace (agent cwds + config stores)", capability)
	}
	return rel, nil
}
