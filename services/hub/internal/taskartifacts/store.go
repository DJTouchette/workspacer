package taskartifacts

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

// Store is created only beneath a host-generated task allocation. os.Root
// confines resolution, including concurrent path replacement, to that root.
// Manifest paths never select a source outside this allocation.
type Store struct {
	root     *os.Root
	manifest Manifest
}

func Open(dir string, m Manifest) (*Store, error) {
	if err := m.Validate(); err != nil {
		return nil, err
	}
	r, err := os.OpenRoot(dir)
	if err != nil {
		return nil, err
	}
	return &Store{r, m}, nil
}

func (s *Store) Close() error { return s.root.Close() }

func (s *Store) entry(index int) (Entry, error) {
	if index < 0 || index >= len(s.manifest.Entries) {
		return Entry{}, fmt.Errorf("artifact not selected for task")
	}
	return s.manifest.Entries[index], nil
}

// Storage names are generated numeric IDs, not the producer's paths. A partial
// is append-only; identical retries succeed and conflicting retries fail.
func (s *Store) Write(index int, offset int64, b []byte) error {
	e, err := s.entry(index)
	if err != nil {
		return err
	}
	if offset < 0 || len(b) > ChunkBytes || offset > e.Size || int64(len(b)) > e.Size-offset {
		return fmt.Errorf("chunk exceeds selected artifact bounds")
	}
	name := fmt.Sprintf("%d.bytes", index)
	f, err := s.root.OpenFile(name, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil || !st.Mode().IsRegular() {
		return fmt.Errorf("invalid staging file")
	}
	if offset < st.Size() {
		overlap := min(int64(len(b)), st.Size()-offset)
		old := make([]byte, overlap)
		if _, err := f.ReadAt(old, offset); err != nil || !bytes.Equal(old, b[:overlap]) {
			return fmt.Errorf("conflicting chunk retry")
		}
		if overlap == int64(len(b)) {
			return nil
		}
		if _, err := f.WriteAt(b[overlap:], offset+overlap); err != nil {
			return err
		}
		return f.Sync()
	}
	if offset != st.Size() {
		return fmt.Errorf("chunk offset mismatch; resume at %d", st.Size())
	}
	if _, err = f.WriteAt(b, offset); err != nil {
		return err
	}
	return f.Sync()
}

func (s *Store) Read(index int, offset int64) ([]byte, error) {
	e, err := s.entry(index)
	if err != nil {
		return nil, err
	}
	if offset < 0 || offset > e.Size {
		return nil, fmt.Errorf("invalid artifact offset")
	}
	f, err := s.root.Open(fmt.Sprintf("%d.bytes", index))
	if err != nil {
		return nil, err
	}
	defer f.Close()
	n := min(int64(ChunkBytes), e.Size-offset)
	b := make([]byte, n)
	_, err = f.ReadAt(b, offset)
	return b, err
}

func (s *Store) Verify() error {
	for i, e := range s.manifest.Entries {
		f, err := s.root.Open(fmt.Sprintf("%d.bytes", i))
		if err != nil {
			return fmt.Errorf("required artifact missing: %s", e.Name)
		}
		b, err := io.ReadAll(io.LimitReader(f, e.Size+1))
		f.Close()
		if err != nil || int64(len(b)) != e.Size || Digest(b) != e.SHA256 {
			return fmt.Errorf("required artifact checksum mismatch: %s", e.Name)
		}
	}
	return nil
}

// Materialize is called only with a generated private directory, before any
// worker is admitted. Files remain outside Git history under .workspacer.
func (s *Store) Materialize(dir string) error {
	if err := s.Verify(); err != nil {
		return err
	}
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	dest, err := os.OpenRoot(dir)
	if err != nil {
		return err
	}
	defer dest.Close()
	for i, e := range s.manifest.Entries {
		if err := dest.MkdirAll(filepath.Dir(e.Name), 0700); err != nil {
			return err
		}
		if existing, err := dest.Open(e.Name); err == nil {
			info, statErr := existing.Stat()
			b, readErr := io.ReadAll(io.LimitReader(existing, e.Size+1))
			existing.Close()
			if statErr != nil || !info.Mode().IsRegular() || readErr != nil || int64(len(b)) != e.Size || Digest(b) != e.SHA256 {
				return fmt.Errorf("materialized artifact changed: %s", e.Name)
			}
			continue
		} else if !os.IsNotExist(err) {
			return err
		}
		temporary := fmt.Sprintf(".handoff-%d.partial", i)
		if info, err := dest.Lstat(temporary); err == nil {
			if !info.Mode().IsRegular() {
				return fmt.Errorf("unexpected artifact staging type")
			}
			if err := dest.Remove(temporary); err != nil {
				return err
			}
		} else if !os.IsNotExist(err) {
			return err
		}
		out, err := dest.OpenFile(temporary, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
		if err != nil {
			return err
		}
		in, err := s.root.Open(fmt.Sprintf("%d.bytes", i))
		if err != nil {
			out.Close()
			return err
		}
		hash := sha256.New()
		var written int64
		written, err = io.Copy(io.MultiWriter(out, hash), io.LimitReader(in, e.Size))
		in.Close()
		if err == nil && (written != e.Size || fmt.Sprintf("%x", hash.Sum(nil)) != e.SHA256) {
			err = fmt.Errorf("artifact changed during materialization")
		}
		if err == nil {
			err = out.Sync()
		}
		out.Close()
		if err != nil {
			return err
		}
		if err := dest.Rename(temporary, e.Name); err != nil {
			return err
		}
	}
	return nil
}
