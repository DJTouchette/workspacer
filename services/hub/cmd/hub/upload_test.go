package main

import (
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// isolateTempDir points os.TempDir at a per-test directory ON EVERY PLATFORM.
// t.Setenv("TMPDIR", …) alone was a silent no-op on Windows — os.TempDir reads
// TMP/TEMP there — so both tests ran against the REAL temp dir: the write
// test's file survived into the refusals test, which then reported it as a
// refusal leaving files behind.
func isolateTempDir(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("TMPDIR", dir)
	t.Setenv("TMP", dir)
	t.Setenv("TEMP", dir)
}

func uploadCall(t *testing.T, name string, data []byte) (map[string]any, error) {
	t.Helper()
	params, _ := json.Marshal(filesUploadParams{
		Name:       name,
		DataBase64: base64.StdEncoding.EncodeToString(data),
	})
	res, err := rpcFilesUpload(params)
	if err != nil {
		return nil, err
	}
	return res.(map[string]any), nil
}

// TestFilesUploadWritesIntoTheLandingPad pins the whole contract: the caller's
// basename is discarded, only the allowlisted extension survives, the file
// lands 0600 in the temp landing pad, and the returned path reads back the
// exact bytes.
func TestFilesUploadWritesIntoTheLandingPad(t *testing.T) {
	isolateTempDir(t)
	payload := []byte{0x89, 'P', 'N', 'G', 0, 1, 2, 3}

	res, err := uploadCall(t, "../../etc/☃ passwd.PNG", payload)
	if err != nil {
		t.Fatalf("upload: %v", err)
	}
	path, _ := res["path"].(string)
	if path == "" {
		t.Fatalf("no path in result: %v", res)
	}
	wantDir := filepath.Join(os.TempDir(), uploadDirName)
	if filepath.Dir(path) != wantDir {
		t.Errorf("wrote to %q, want inside %q — the caller's name must never steer the directory", path, wantDir)
	}
	if !strings.HasSuffix(path, ".png") {
		t.Errorf("extension not normalized to lowercase allowlisted form: %q", path)
	}
	if base := filepath.Base(path); strings.ContainsAny(base, "☃ .") && strings.Count(base, ".") != 1 {
		t.Errorf("caller basename leaked into %q", base)
	}
	got, err := os.ReadFile(path)
	if err != nil || string(got) != string(payload) {
		t.Errorf("read back %q err=%v, want the uploaded bytes", got, err)
	}
	// POSIX only: NTFS has no mode bits — Go reports 0666 for any writable
	// file on Windows regardless of what the create call asked for.
	if fi, _ := os.Stat(path); runtime.GOOS != "windows" && fi.Mode().Perm() != 0o600 {
		t.Errorf("mode %v, want 0600", fi.Mode().Perm())
	}
	if size, _ := res["size"].(int); size != len(payload) {
		t.Errorf("size %v, want %d", res["size"], len(payload))
	}
}

func TestFilesUploadRefusals(t *testing.T) {
	isolateTempDir(t)
	cases := []struct {
		label string
		raw   string // raw JSON params
		want  string // error substring
	}{
		{"disallowed extension", `{"name":"payload.sh","dataBase64":"aGk="}`, "not allowed"},
		{"no extension", `{"name":"payload","dataBase64":"aGk="}`, "not allowed"},
		{"empty payload", `{"name":"a.png","dataBase64":""}`, "required"},
		{"invalid base64", `{"name":"a.png","dataBase64":"!!!"}`, "not valid base64"},
	}
	for _, c := range cases {
		if _, err := rpcFilesUpload(json.RawMessage(c.raw)); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%s: err=%v, want substring %q", c.label, err, c.want)
		}
	}

	// Oversize is refused BEFORE decoding: an encoded length just past the cap.
	big := strings.Repeat("A", (maxUploadBytes/3+2)*4)
	if _, err := rpcFilesUpload(json.RawMessage(`{"name":"a.png","dataBase64":"` + big + `"}`)); err == nil ||
		!strings.Contains(err.Error(), "exceeds") {
		t.Errorf("oversize: err=%v, want size refusal", err)
	}
	// Nothing may have been written by any refusal.
	if entries, _ := os.ReadDir(filepath.Join(os.TempDir(), uploadDirName)); len(entries) != 0 {
		t.Errorf("refused uploads left %d files behind", len(entries))
	}
}
