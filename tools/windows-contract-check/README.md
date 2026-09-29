Run `make check-hub-rust-windows-platform` from the repository root after
installing the `x86_64-pc-windows-gnu` Rust target. The crate imports the production
Windows source files directly, including their tests. It checks Windows API and
type usage without requiring the full hub's C cross-compiler dependencies.

Run the tests on Windows to verify ACL and process behavior; cross-compilation
alone does not establish those runtime properties.
