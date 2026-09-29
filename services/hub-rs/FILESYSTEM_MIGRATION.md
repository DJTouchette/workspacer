# Filesystem parity review

Authenticated file access remains ambient: an absolute path outside the picker
home or a live session cwd is valid. Paths are resolved component by component,
with symlinks resolved before later parent traversal. Selected library, session,
repository and replay objects separately enforce containment. The retained Go
secret-path helpers are not the active authenticated filesystem policy.

The filesystem handler audit preserves directory-only picker ordering, hidden
picker entries, literal canonical spellings, regular-file/5 MiB/NUL/UTF-8 read
checks, parent creation, write result shape, and directory-first file-tree
ordering. Git ignore filtering uses the original fixed configuration arguments
and NUL-delimited names; executable argv and newline/non-ASCII stdin are tested.
Missing Git or non-repository errors leave the listing unfiltered. Cancellation
owns and terminates the helper instead of leaving an independent process alive.

The review found three Rust defaults that needed correction: an empty Path was
accepted as a containment prefix; default creation modes differed from Go under
umask zero; and malformed picker path values silently became a home-directory
request. Empty roots now refuse, new directories/files request 0755/0644, and path
parameters are checked before access. Existing file modes are preserved.
ASCII-blank picker paths default to home while absolute trailing spaces remain
literal. A missing home returns an empty supervisor-home receipt without creating
state relative to the process working directory.

Rust bounds bytes read after opening, checks opened file identity and refuses
special files; these are stricter than the old stat-then-unbounded-read behavior.
They do not promise atomic exclusion of all parent-directory replacement races.
Supervisor-home directory creation errors are surfaced rather than swallowed.
Invalid filename bytes are rendered lossily for JSON as before; text file content
itself is never decoded lossily. Platform filesystem behavior still requires the
Windows/macOS CI gates, separately from local Linux evidence.

The production `fsguard.go` review also traced its non-authorization live-cwd
consumers. Library polling now observes external writes through redacted public
projections and leaves stopped projects; dispatch readiness uses process
liveness and canonical directory identity. The owned engine uses its live
snapshot map instead of the old process-global HTTP cwd cache. Catalog polling
retains its last known roots when a bounded inventory read fails. Details and
exact source hashes are in `reviews/brain-live-directories.json`.
