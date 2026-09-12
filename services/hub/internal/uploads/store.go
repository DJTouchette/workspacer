// Shared attachment spill. Execute in the identity that runs the agent.
package uploads

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func DirName() string { return "workspacer-uploads-" + strconv.Itoa(os.Getuid()) }

// MaxBytes caps the DECODED payload. The bus reads frames up to 64 MiB,
// so 24 MiB of raw bytes (~32 MiB base64) leaves comfortable envelope room.
const MaxBytes = 24 << 20

var uploadExts = map[string]bool{
	".png": true, ".jpg": true, ".jpeg": true, ".gif": true, ".webp": true, ".pdf": true,
}

type Params struct {
	// Name is advisory: only its extension survives, and only allowlisted ones.
	Name       string `json:"name"`
	DataBase64 string `json:"dataBase64"`
}

// rpcFilesUpload writes the payload to the OS temp dir (same lifetime class as
// the desktop's pasted-screenshot spill, os.tmpdir()/workspacer-pasted) and
// returns the absolute path for the caller to reference in a message.
func Store(params json.RawMessage) (any, error) {
	var p Params
	if err := json.Unmarshal(params, &p); err != nil {
		return nil, fmt.Errorf("files.upload: bad params: %w", err)
	}
	if p.DataBase64 == "" {
		return nil, fmt.Errorf("files.upload: dataBase64 is required")
	}
	// Reject on the cheap encoded length before decoding anything.
	if len(p.DataBase64) > (MaxBytes/3+1)*4 {
		return nil, fmt.Errorf("files.upload: payload exceeds %d MiB", MaxBytes>>20)
	}
	data, err := base64.StdEncoding.DecodeString(p.DataBase64)
	if err != nil {
		return nil, fmt.Errorf("files.upload: dataBase64 is not valid base64")
	}
	if len(data) == 0 {
		return nil, fmt.Errorf("files.upload: empty payload")
	}
	if len(data) > MaxBytes {
		return nil, fmt.Errorf("files.upload: payload exceeds %d MiB", MaxBytes>>20)
	}
	ext := strings.ToLower(filepath.Ext(p.Name))
	if !uploadExts[ext] {
		return nil, fmt.Errorf("files.upload: extension %q not allowed (png, jpg, jpeg, gif, webp, pdf)", ext)
	}
	dir := Directory()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("files.upload: %w", err)
	}
	var rnd [4]byte
	if _, err := rand.Read(rnd[:]); err != nil {
		return nil, fmt.Errorf("files.upload: %w", err)
	}
	name := fmt.Sprintf("m-%d-%s%s", time.Now().UnixMilli(), hex.EncodeToString(rnd[:]), ext)
	path := filepath.Join(dir, name)
	file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return nil, fmt.Errorf("files.upload: %w", err)
	}
	_, writeErr := file.Write(data)
	closeErr := file.Close()
	if writeErr != nil {
		_ = os.Remove(path)
		return nil, writeErr
	}
	if closeErr != nil {
		_ = os.Remove(path)
		return nil, closeErr
	}
	return map[string]any{"path": path, "size": len(data)}, nil
}

func Directory() string { return filepath.Join(os.TempDir(), DirName()) }
