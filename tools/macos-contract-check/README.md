# macOS ownership API compile probe

`cargo check --locked --manifest-path tools/macos-contract-check/Cargo.toml --target aarch64-apple-darwin --tests`
checks the actual claudemon process-group source against its older independently
locked libc ABI. Install that Rust target with `rustup target add aarch64-apple-darwin`.
No Apple SDK or linker is needed. This catches missing bindings on Linux; it is
not runtime evidence. The preview workflow runs the actual ownership and PTY
regressions on macOS, plus the shared Hub suite.
