# Retained bus latency guard

The executable budget from Go `internal/bus/bench_test.go` is retained by
`tests/bus_latency.rs`. It measures 2,000 sequential publish-to-delivery hops
through the real Rust hub WebSocket transport, using the original 200-turn
mature-session snapshot fields and text. It then measures 500 text-frame echoes
through a bare loopback WebSocket with the same payload byte count.

The retained comparison is `hub p99 - bare echo p99 <= 5ms`. The percentile index
is the same integer `len * 99 / 100` rule as Go. The bare echo p99 must be at most
2.5ms; otherwise the runner cannot resolve this budget and the result is explicitly
`unmeasurable`. Windows remains unmeasurable because the legacy source documents
scheduler jitter that its echo floor does not predict. Instrumented builds must
set `WORKSPACER_BUS_LATENCY_INSTRUMENTED=1` and also report `unmeasurable`.

Run the measured guard only with the optimized release profile:

```sh
cargo test --locked --release --manifest-path services/hub-rs/Cargo.toml \
  --test bus_latency -- --ignored --nocapture
```

The root-maintained Makefile/CI entry invokes that command as `test-hub-latency`.
The integration test intentionally refuses a debug invocation instead of comparing
unoptimized Rust to the optimized Go compiler's old result. Ordinary debug test
runs execute the workload/threshold decision checks and visibly ignore the actual
measurement. A normal debug suite alone is therefore not performance evidence.

Each measured run prints a JSON receipt with status, payload bytes, turn/sample
counts, p50/p99, bare echo floor and the baseline-adjusted share. A slow hub on a
measurable runner fails the test. An unmeasurable receipt is not a performance pass.
WebSocket reads/writes have deadlines, and received payloads are checked so a fast
error response or dropped message cannot be recorded as a successful hop.

This is a local transport budget, not a whole-application rendering, agent latency,
throughput, memory or cross-language speedup claim. No provider process or user
state is accessed by the fixture.
