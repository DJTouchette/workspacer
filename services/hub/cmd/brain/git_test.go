package main

// The parsers and the argv builder, ported alongside the code they belong to.
//
// TWIN: apps/desktop/src/main/services/gitService.test.ts. The two providers
// answer the same bus methods, so a parser that disagrees is a Review pane that
// shows a different file list depending on who happened to answer.

import (
	"reflect"
	"strings"
	"testing"
)

func TestParseGitPorcelain(t *testing.T) {
	t.Run("modified and untracked", func(t *testing.T) {
		got := parseGitPorcelain(" M src/a.ts\x00?? new.txt\x00")
		want := []gitFileStatus{
			{Path: "src/a.ts", Staged: " ", Unstaged: "M"},
			{Path: "new.txt", Staged: "?", Unstaged: "?"},
		}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("got %+v, want %+v", got, want)
		}
	})

	// The rename format that is NOT the numstat one: `-z` puts the source path
	// in its own NUL-terminated token rather than inline after " -> ", which is
	// why a fix in parseGitNumstatPath does not fix this one.
	t.Run("rename takes its source from the next token", func(t *testing.T) {
		got := parseGitPorcelain("R  new/path.ts\x00old/path.ts\x00 M other.ts\x00")
		want := []gitFileStatus{
			{Path: "new/path.ts", OrigPath: "old/path.ts", Staged: "R", Unstaged: " "},
			{Path: "other.ts", Staged: " ", Unstaged: "M"},
		}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("got %+v, want %+v", got, want)
		}
	})

	t.Run("an entry shorter than XY<space><path> is skipped", func(t *testing.T) {
		if got := parseGitPorcelain(" M \x00 M a\x00"); len(got) != 1 || got[0].Path != "a" {
			t.Fatalf("got %+v", got)
		}
	})

	t.Run("unmerged codes survive verbatim", func(t *testing.T) {
		// isUnmergedStatus (gitQueries.ts) buckets the Conflicts section off
		// these two characters; mangling either one empties that section.
		got := parseGitPorcelain("UU both.ts\x00")
		if len(got) != 1 || got[0].Staged != "U" || got[0].Unstaged != "U" {
			t.Fatalf("got %+v", got)
		}
	})
}

func TestParseGitNumstatPath(t *testing.T) {
	cases := []struct{ in, want string }{
		{"src/a.ts", "src/a.ts"},
		{"old.ts => new.ts", "new.ts"},
		{"src/{old => new}/a.ts", "src/new/a.ts"},
		{"src/{ => sub}/a.ts", "src/sub/a.ts"},
		{"src/{old => }/a.ts", "src/a.ts"},
	}
	for _, c := range cases {
		if got := parseGitNumstatPath(c.in); got != c.want {
			t.Errorf("parseGitNumstatPath(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestParseGitNumstat(t *testing.T) {
	got := parseGitNumstat("3\t1\tsrc/a.ts\n-\t-\timg.png\n12\t0\tsrc/{old => new}/b.ts\n")
	if len(got) != 3 {
		t.Fatalf("want 3 rows, got %+v", got)
	}
	if got[0].Path != "src/a.ts" || got[0].Added == nil || *got[0].Added != 3 || *got[0].Deleted != 1 {
		t.Errorf("row 0: %+v", got[0])
	}
	// A binary file prints "-": it has to reach the renderer as null, not 0.
	if got[1].Added != nil || got[1].Deleted != nil {
		t.Errorf("binary row must carry null counts, got %+v", got[1])
	}
	if got[2].Path != "src/new/b.ts" {
		t.Errorf("row 2 path: %q", got[2].Path)
	}
}

func TestParseGitBranchHeader(t *testing.T) {
	str := func(s string) *string { return &s }
	cases := []struct {
		name     string
		in       string
		upstream *string
		ahead    int
		behind   int
	}{
		{"no upstream", "## master", nil, 0, 0},
		{"upstream, in sync", "## master...origin/master", str("origin/master"), 0, 0},
		{"ahead and behind", "## m...origin/m [ahead 1, behind 2]", str("origin/m"), 1, 2},
		{"ahead only", "## m...origin/m [ahead 3]", str("origin/m"), 3, 0},
		// A gone upstream is treated as none: plain `git push` cannot reach it,
		// so the Push button must not be greyed out on an ahead count of 0.
		{"gone upstream", "## m...origin/m [gone]", nil, 0, 0},
		{"detached", "## HEAD (no branch)", nil, 0, 0},
		{"unborn", "## No commits yet on main", nil, 0, 0},
		{"not a header", "M a.ts", nil, 0, 0},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			up, ahead, behind := parseGitBranchHeader(c.in)
			if (up == nil) != (c.upstream == nil) || (up != nil && *up != *c.upstream) {
				t.Errorf("upstream = %v, want %v", up, c.upstream)
			}
			if ahead != c.ahead || behind != c.behind {
				t.Errorf("ahead/behind = %d/%d, want %d/%d", ahead, behind, c.ahead, c.behind)
			}
		})
	}
}

func TestParseGitLog(t *testing.T) {
	got := parseGitLog("abc123\x00first subject\x001700000000\ndef456\x00second\x001700000001\n")
	if len(got) != 2 {
		t.Fatalf("want 2 commits, got %+v", got)
	}
	if got[0].Hash != "abc123" || got[0].Subject != "first subject" || got[0].AuthoredAt != 1700000000 {
		t.Errorf("commit 0: %+v", got[0])
	}
	// A row whose timestamp is not a number is dropped rather than surfaced as
	// an epoch-zero commit.
	if got := parseGitLog("abc\x00s\x00notanumber\n"); len(got) != 0 {
		t.Errorf("want no commits, got %+v", got)
	}
}

// gitArgs is a security primitive, not a formatting helper: without the prefix a
// caller-written .git/config executes commands, and without --no-ext-diff
// `diff.external` does (the one exec key the -c prefix structurally cannot
// neutralize).
func TestGitArgsCarriesTheNoExecPrefixAndTheDiffOptOut(t *testing.T) {
	t.Run("prefix is in front of the subcommand", func(t *testing.T) {
		got := gitArgs([]string{"status", "--porcelain"})
		if got[0] != "-c" || !strings.HasPrefix(got[1], "core.fsmonitor=") {
			t.Fatalf("got %v", got)
		}
		if idx := indexOf(got, "status"); idx != len(gitNoExecConfig()) {
			t.Fatalf("subcommand at %d, want %d: %v", idx, len(gitNoExecConfig()), got)
		}
	})

	t.Run("--no-ext-diff lands right after a diff subcommand", func(t *testing.T) {
		got := gitArgs([]string{"diff", "--staged"})
		i := indexOf(got, "diff")
		if i < 0 || got[i+1] != "--no-ext-diff" || got[i+2] != "--staged" {
			t.Fatalf("got %v", got)
		}
	})

	// The caller's own leading `-c` (gitNumstat's core.quotepath=false) must not
	// be mistaken for the subcommand, or --no-ext-diff never gets inserted.
	t.Run("a caller-supplied -c pair is skipped when finding the subcommand", func(t *testing.T) {
		got := gitArgs([]string{"-c", "core.quotepath=false", "diff", "--numstat"})
		i := indexOf(got, "diff")
		if i < 0 || got[i+1] != "--no-ext-diff" {
			t.Fatalf("got %v", got)
		}
	})

	t.Run("a non-diff subcommand gets no --no-ext-diff", func(t *testing.T) {
		if got := gitArgs([]string{"rev-parse", "--show-toplevel"}); indexOf(got, "--no-ext-diff") != -1 {
			t.Fatalf("got %v", got)
		}
	})
}

func indexOf(ss []string, want string) int {
	for i, s := range ss {
		if s == want {
			return i
		}
	}
	return -1
}

// The diff family is a TWIN of DIFF_FAMILY in gitExec.ts. A subcommand covered
// on one side and not the other is a capability that honours `diff.external`
// depending on which provider answered.
func TestGitDiffFamilyMatchesTheDesktopTwin(t *testing.T) {
	src := string(mustReadRepoFile(t, "apps", "desktop", "src", "main", "lib", "gitExec.ts"))
	start := strings.Index(src, "const DIFF_FAMILY = new Set([")
	if start == -1 {
		t.Fatal("could not find DIFF_FAMILY in gitExec.ts — this parity test has stopped comparing anything")
	}
	end := strings.Index(src[start:], "])")
	if end == -1 {
		t.Fatal("DIFF_FAMILY is not terminated by `])` — update this parser")
	}
	ts := map[string]bool{}
	for _, line := range strings.Split(src[start:start+end], "\n") {
		line = strings.Trim(strings.TrimSpace(line), ",")
		if strings.HasPrefix(line, "'") && strings.HasSuffix(line, "'") {
			ts[strings.Trim(line, "'")] = true
		}
	}
	if len(ts) == 0 {
		t.Fatal("parsed zero subcommands out of DIFF_FAMILY")
	}
	for name := range gitDiffFamily {
		if !ts[name] {
			t.Errorf("gitDiffFamily has %q and the desktop twin does not", name)
		}
		delete(ts, name)
	}
	for name := range ts {
		t.Errorf("gitExec.ts's DIFF_FAMILY has %q and gitDiffFamily does not", name)
	}
}
