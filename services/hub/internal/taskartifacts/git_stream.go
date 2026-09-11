package taskartifacts

import (
	"bufio"
	"bytes"
	"context"
	"fmt"
	"io"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

type treeObject struct {
	oid  string
	size int64
}

// One streaming Git process avoids thousands of process launches on Windows.
// Per-blob and aggregate sizes are admitted before reading data; no tree-sized
// buffer is allocated and no filters/textconv or lazy network fetch is enabled.
func verifyBlobStream(ctx context.Context, repo string, objects []treeObject) error {
	if len(objects) == 0 {
		return nil
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	var input strings.Builder
	for _, object := range objects {
		input.WriteString(object.oid)
		input.WriteByte('\n')
	}
	cmd := exec.CommandContext(ctx, "git", append(gitBaseArgs(""), "cat-file", "--batch")...)
	cmd.Dir = repo
	cmd.Env = gitEnvironment()
	cmd.Stdin = strings.NewReader(input.String())
	cmd.Stderr = &boundedOutput{limit: 8192}
	out, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	if err := cmd.Start(); err != nil {
		return err
	}
	waited := false
	defer func() {
		if !waited {
			cancel()
			_ = cmd.Wait()
		}
	}()
	reader := bufio.NewReaderSize(out, 64<<10)
	for _, object := range objects {
		line, err := reader.ReadString('\n')
		if err != nil {
			return fmt.Errorf("incomplete Git blob header")
		}
		fields := strings.Fields(line)
		if len(fields) != 3 || fields[0] != object.oid || fields[1] != "blob" {
			return fmt.Errorf("Git blob identity mismatch")
		}
		size, err := strconv.ParseInt(fields[2], 10, 64)
		if err != nil || size != object.size {
			return fmt.Errorf("Git blob size changed")
		}
		prefix := make([]byte, min(size, 64))
		if _, err := io.ReadFull(reader, prefix); err != nil {
			return err
		}
		if bytes.HasPrefix(prefix, []byte("version https://git-lfs.github.com/spec/v1")) {
			return fmt.Errorf("unsupported unresolved LFS input")
		}
		if _, err := io.CopyN(io.Discard, reader, size-int64(len(prefix))); err != nil {
			return err
		}
		boundary, err := reader.ReadByte()
		if err != nil || boundary != '\n' {
			return fmt.Errorf("invalid Git blob boundary")
		}
	}
	err = cmd.Wait()
	waited = true
	if err != nil {
		return fmt.Errorf("Git blob verification failed or timed out")
	}
	return nil
}
