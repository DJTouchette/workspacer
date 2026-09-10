// Package taskartifacts owns bounded task bytes. A digest is an integrity
// check, never authority: callers must resolve the authenticated task first.
package taskartifacts

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"path"
	"regexp"
	"strings"
)

const Version = 1
const ChunkBytes = 256 << 10
const FileBytes = 16 << 20
const TaskBytes = 64 << 20
const MaxFiles = 128

type Entry struct {
	Name   string `json:"name"`
	Kind   string `json:"kind"`
	Size   int64  `json:"size"`
	SHA256 string `json:"sha256"`
}

type Manifest struct {
	Version      int     `json:"version"`
	Task         string  `json:"task"`
	Origin       string  `json:"origin"`
	Producer     string  `json:"producer"`
	Commit       string  `json:"commit"`
	ObjectFormat string  `json:"objectFormat"`
	Entries      []Entry `json:"entries"`
}

var ID = regexp.MustCompile(`^[a-z0-9][a-z0-9_-]{15,127}$`)
var hexDigest = regexp.MustCompile(`^[0-9a-f]{64}$`)
var portableComponent = regexp.MustCompile(`^[A-Za-z0-9_][A-Za-z0-9_. -]*$`)
var reserved = regexp.MustCompile(`(?i)^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)`)

// V1 deliberately supports portable ASCII names; Unicode normalization and
// platform aliases are refused rather than guessed differently by each host.
func ValidName(name string) error {
	if len(name) == 0 || len(name) > 200 || path.Clean(name) != name {
		return fmt.Errorf("unsupported artifact path %q", name)
	}
	for _, part := range strings.Split(name, "/") {
		if !portableComponent.MatchString(part) || strings.HasSuffix(part, ".") || strings.HasSuffix(part, " ") || reserved.MatchString(part) || strings.EqualFold(part, ".git") {
			return fmt.Errorf("unsupported artifact path %q", name)
		}
	}
	return nil
}

func Digest(b []byte) string { sum := sha256.Sum256(b); return hex.EncodeToString(sum[:]) }

func recordPortablePath(spelling map[string]string, name string) error {
	parts := strings.Split(name, "/")
	for i := range parts {
		prefix := strings.Join(parts[:i+1], "/")
		key := strings.ToLower(prefix)
		if prior, ok := spelling[key]; ok && prior != prefix {
			return fmt.Errorf("path component case collision: %s", name)
		}
		spelling[key] = prefix
	}
	return nil
}

func (m Manifest) Validate() error {
	if m.Version != Version || !ID.MatchString(m.Task) || !ID.MatchString(m.Origin) || !ID.MatchString(m.Producer) {
		return fmt.Errorf("unsupported manifest version or task ownership")
	}
	n := 40
	if m.ObjectFormat == "sha256" {
		n = 64
	} else if m.ObjectFormat != "sha1" {
		return fmt.Errorf("unsupported Git object format")
	}
	if len(m.Commit) != n || strings.Trim(m.Commit, "0123456789abcdef") != "" {
		return fmt.Errorf("exact commit required")
	}
	if len(m.Entries) > MaxFiles {
		return fmt.Errorf("artifact count exceeds %d", MaxFiles)
	}
	seen := map[string]bool{}
	spelling := map[string]string{}
	var total int64
	for _, e := range m.Entries {
		if err := ValidName(e.Name); err != nil {
			return err
		}
		if err := recordPortablePath(spelling, e.Name); err != nil {
			return err
		}
		key := strings.ToLower(e.Name)
		if seen[key] {
			return fmt.Errorf("artifact path collision: %s", e.Name)
		}
		for prior := range seen {
			if strings.HasPrefix(key, prior+"/") || strings.HasPrefix(prior, key+"/") {
				return fmt.Errorf("artifact directory collision")
			}
		}
		seen[key] = true
		if e.Size < 0 || e.Size > FileBytes || !hexDigest.MatchString(e.SHA256) {
			return fmt.Errorf("invalid artifact size or digest: %s", e.Name)
		}
		switch e.Kind {
		case "report", "criteria", "image", "log":
		default:
			return fmt.Errorf("unsupported artifact kind")
		}
		ext := strings.ToLower(path.Ext(e.Name))
		allowed := (e.Kind == "report" || e.Kind == "criteria") && (ext == ".md" || ext == ".txt") || e.Kind == "log" && (ext == ".log" || ext == ".txt" || ext == ".json" || ext == ".jsonl") || e.Kind == "image" && (ext == ".png" || ext == ".jpg" || ext == ".jpeg" || ext == ".gif" || ext == ".webp")
		if !allowed {
			return fmt.Errorf("artifact kind and safe file extension disagree: %s", e.Name)
		}
		total += e.Size
	}
	if total > TaskBytes {
		return fmt.Errorf("task artifacts exceed %d bytes", TaskBytes)
	}
	return nil
}

func (m Manifest) Seal() (string, error) {
	if err := m.Validate(); err != nil {
		return "", err
	}
	b, err := json.Marshal(m)
	if err != nil {
		return "", err
	}
	return Digest(b), nil
}
