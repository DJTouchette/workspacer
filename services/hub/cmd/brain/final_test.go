package main

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestEncodeDirName(t *testing.T) {
	cases := map[string]string{
		"/foo/bar":  "-foo-bar",
		"/foo/bar/": "-foo-bar",
		`C:\foo`:    "C--foo",
		"/a:b/c":    "-a-b-c",
	}
	for in, want := range cases {
		if got := encodeDirName(in); got != want {
			t.Errorf("encodeDirName(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestSessionsForDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	cwd := "/home/u/proj"
	dir := filepath.Join(home, ".claude", "projects", encodeDirName(cwd))
	writeFile(t, filepath.Join(dir, "11111111-1111-1111-1111-111111111111.jsonl"),
		`{"type":"user","message":{"content":"Hello there friend"}}`+"\n")
	writeFile(t, filepath.Join(dir, "22222222-2222-2222-2222-222222222222.jsonl"),
		`{"type":"summary","summary":"My session name"}`+"\n")
	writeFile(t, filepath.Join(dir, "agent-33333333.jsonl"),
		`{"type":"user","message":{"content":"subagent"}}`+"\n")

	got := listClaudeSessionsForDir(cwd)
	if len(got) != 2 {
		t.Fatalf("expected 2 sessions (subagent skipped), got %d: %+v", len(got), got)
	}
	summaries := map[string]string{}
	for _, s := range got {
		summaries[s.SessionID] = s.Summary
	}
	if summaries["11111111-1111-1111-1111-111111111111"] != "Hello there friend" {
		t.Errorf("user-message summary wrong: %q", summaries["11111111-1111-1111-1111-111111111111"])
	}
	if summaries["22222222-2222-2222-2222-222222222222"] != "My session name" {
		t.Errorf("summary-entry wrong: %q", summaries["22222222-2222-2222-2222-222222222222"])
	}
}

func TestListEntriesHidesGitAndIgnored(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not available")
	}
	dir := t.TempDir()
	if err := exec.Command("git", "-C", dir, "init").Run(); err != nil {
		t.Skipf("git init failed: %v", err)
	}
	writeFile(t, filepath.Join(dir, ".gitignore"), "ignored.txt\n")
	writeFile(t, filepath.Join(dir, "visible.txt"), "v")
	writeFile(t, filepath.Join(dir, "ignored.txt"), "i")
	if err := os.Mkdir(filepath.Join(dir, "sub"), 0o755); err != nil {
		t.Fatal(err)
	}

	res, err := listEntries(dir)
	if err != nil {
		t.Fatal(err)
	}
	names := map[string]bool{}
	for _, e := range res.Entries {
		names[e.Name] = true
	}
	if names[".git"] || names["ignored.txt"] {
		t.Fatalf(".git and ignored files must be hidden: %+v", res.Entries)
	}
	if !names["visible.txt"] || !names["sub"] || !names[".gitignore"] {
		t.Fatalf("expected visible.txt, sub, .gitignore: %+v", res.Entries)
	}
	if !res.Entries[0].IsDir { // directories sort first
		t.Errorf("directories should sort first, got %+v", res.Entries[0])
	}
}

func TestReadTextFileGuards(t *testing.T) {
	dir := t.TempDir()
	good := filepath.Join(dir, "ok.txt")
	writeFile(t, good, "hello")
	res, err := readTextFile(good)
	if err != nil || res.Contents != "hello" || res.Size != 5 {
		t.Fatalf("read good file failed: %+v err %v", res, err)
	}

	binary := filepath.Join(dir, "bin")
	if err := os.WriteFile(binary, []byte{0x68, 0x00, 0x69}, 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := readTextFile(binary); err == nil {
		t.Fatal("expected binary file to be rejected")
	}
}

func TestSupervisorHome(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	got := supervisorHome()
	want := filepath.Join(home, ".workspacer")
	if got != want {
		t.Fatalf("supervisorHome = %q, want %q", got, want)
	}
	if st, err := os.Stat(want); err != nil || !st.IsDir() {
		t.Fatalf("supervisor home not created: %v", err)
	}
}

func TestSearchProject(t *testing.T) {
	if _, err := exec.LookPath("rg"); err != nil {
		t.Skip("ripgrep not available")
	}
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "a.txt"), "alpha\nfind-me here\nbeta\n")
	writeFile(t, filepath.Join(dir, "b.txt"), "nothing\n")

	res, err := searchProject(context.Background(), searchOpts{Query: "find-me", Cwd: dir})
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Results) != 1 || len(res.Results[0].Matches) != 1 {
		t.Fatalf("expected one match in one file, got %+v", res.Results)
	}
	m := res.Results[0].Matches[0]
	if m.Line != 2 || m.Column != 1 {
		t.Errorf("match position = line %d col %d, want line 2 col 1", m.Line, m.Column)
	}
}
