// files.upload — land bytes from a remote client (the /m PWA's photo
// attachments) on THIS machine's disk, so a chat message can reference them as
// a plain [Image: /path] prefix exactly like the desktop composer does.
//
// A bus local rather than an HTTP route for two reasons:
//   - federation: hub:<peer>/files.upload forwards like any other call, so a
//     phone paired with one hub can attach a photo to an agent running on a
//     peer machine — the bytes are written by the peer's own hub;
//   - auth: the bus already resolves the caller's tier; the HTTP `guard` is
//     host-or-operator only and would 401 every triage-scoped phone.
//
// The caller controls only the extension (allowlisted); the directory and
// basename are ours, so this is not a path-confinement surface like fs.write.
package main

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const uploadDirName = "workspacer-uploads"

// maxUploadBytes caps the DECODED payload. The bus reads frames up to 64 MiB,
// so 24 MiB of raw bytes (~32 MiB base64) leaves comfortable envelope room.
const maxUploadBytes = 24 << 20

var uploadExts = map[string]bool{
	".png": true, ".jpg": true, ".jpeg": true, ".gif": true, ".webp": true, ".pdf": true,
}

type filesUploadParams struct {
	// Name is advisory: only its extension survives, and only allowlisted ones.
	Name       string `json:"name"`
	DataBase64 string `json:"dataBase64"`
}

// rpcFilesUpload writes the payload to the OS temp dir (same lifetime class as
// the desktop's pasted-screenshot spill, os.tmpdir()/workspacer-pasted) and
// returns the absolute path for the caller to reference in a message.
func rpcFilesUpload(params json.RawMessage) (any, error) {
	var p filesUploadParams
	if err := json.Unmarshal(params, &p); err != nil {
		return nil, fmt.Errorf("files.upload: bad params: %w", err)
	}
	if p.DataBase64 == "" {
		return nil, fmt.Errorf("files.upload: dataBase64 is required")
	}
	// Reject on the cheap encoded length before decoding anything.
	if len(p.DataBase64) > (maxUploadBytes/3+1)*4 {
		return nil, fmt.Errorf("files.upload: payload exceeds %d MiB", maxUploadBytes>>20)
	}
	data, err := base64.StdEncoding.DecodeString(p.DataBase64)
	if err != nil {
		return nil, fmt.Errorf("files.upload: dataBase64 is not valid base64")
	}
	if len(data) == 0 {
		return nil, fmt.Errorf("files.upload: empty payload")
	}
	if len(data) > maxUploadBytes {
		return nil, fmt.Errorf("files.upload: payload exceeds %d MiB", maxUploadBytes>>20)
	}
	ext := strings.ToLower(filepath.Ext(p.Name))
	if !uploadExts[ext] {
		return nil, fmt.Errorf("files.upload: extension %q not allowed (png, jpg, jpeg, gif, webp, pdf)", ext)
	}
	dir := filepath.Join(os.TempDir(), uploadDirName)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("files.upload: %w", err)
	}
	var rnd [4]byte
	if _, err := rand.Read(rnd[:]); err != nil {
		return nil, fmt.Errorf("files.upload: %w", err)
	}
	name := fmt.Sprintf("m-%d-%s%s", time.Now().UnixMilli(), hex.EncodeToString(rnd[:]), ext)
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		return nil, fmt.Errorf("files.upload: %w", err)
	}
	return map[string]any{"path": path, "size": len(data)}, nil
}
