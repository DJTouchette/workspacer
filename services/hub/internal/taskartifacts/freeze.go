package taskartifacts

import (
	"fmt"
	"io"
	"os"
)

// ReadSelected reads only a selected regular file under a trusted producer
// root. A report's absolute paths or URLs are never followed as selections.
// Two reads plus identity/metadata checks catch changing inputs; the frozen
// bytes, not a later read of the source, define the immutable manifest.
func ReadSelected(root *os.Root, name string) ([]byte, error) {
	if err := ValidName(name); err != nil {
		return nil, err
	}
	info, err := root.Lstat(name)
	if err != nil || !info.Mode().IsRegular() {
		return nil, fmt.Errorf("selected artifact must be a regular file")
	}
	f, err := root.Open(name)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	before, err := f.Stat()
	if err != nil || !os.SameFile(info, before) || !singleLink(f, before) || before.Size() > FileBytes {
		return nil, fmt.Errorf("selected artifact identity, links or size unsupported")
	}
	b, err := io.ReadAll(io.LimitReader(f, FileBytes+1))
	if err != nil || len(b) > FileBytes {
		return nil, fmt.Errorf("artifact exceeds file limit")
	}
	if _, err = f.Seek(0, 0); err != nil {
		return nil, err
	}
	second, err := io.ReadAll(io.LimitReader(f, FileBytes+1))
	after, statErr := f.Stat()
	if err != nil || statErr != nil || before.Size() != after.Size() || !before.ModTime().Equal(after.ModTime()) || Digest(b) != Digest(second) {
		return nil, fmt.Errorf("selected artifact changed during freeze")
	}
	return b, nil
}
