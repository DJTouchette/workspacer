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

## Historical optimized checkpoint

[Linux CI run 36627270467](https://github.com/DJTouchette/workspacer/actions/runs/36627270467/job/109607363818)
on `a271e4ff32d0283ce6c43879014ac5455fe0026e` executed the guard successfully:
200 turns, 38,005 payload bytes, 2,000 hub samples and 500 echo samples. Hub p50
was 406µs, hub p99 was 542µs, the echo p99 was 165µs, and the measured hub share
was 377µs against the retained 5,000µs budget. The receipt reported `pass`, with
zero ignored tests in that optimized invocation. This is one source-revision
checkpoint; the dedicated CI job continues to guard subsequent pushes.

## Current regression and transport correction

[Linux CI run 36637635374](https://github.com/DJTouchette/workspacer/actions/runs/36637635374/job/109642058103)
on `a7e11447b5a8a9097401786e46b2d3b1e47f0048` **failed** the same real budget:
hub p50 437µs, hub p99 40,037µs, echo p99 143µs and adjusted share 39,894µs.
It ran 2,000/500 samples with the same 200-turn, 38,005-byte payload. This failure
remains the current optimized receipt until a corrected run passes.

The pinned transport sources show a configuration discrepancy. Go's TCP
constructor enables TCP_NODELAY for both dialed and accepted sockets. Rust's
production outbound Client already disables Nagle, but the benchmark used
`connect_async`, whose default does not. The pinned Axum 0.7 accepted listener
also leaves TCP_NODELAY unset unless its builder is configured explicitly.
The hub HTTP/bus, MCP and claudemon API/hook listeners now set it; benchmark
clients and bare echo sockets use the same option. Other discovered Axum serve
calls are isolated test/example servers, not additional production listeners.

`tests/tcp_options.rs` provides Linux accepted-socket configuration checks using
kernel socket options, including a deliberate false/true control. The benchmark
asserts its client/echo options and retains its existing debug payload exchange.
These are configuration/functional evidence, not optimized timing evidence. The
roughly 40ms stalls are consistent with Nagle/delayed ACK behavior, but that causal
interpretation does not replace the next unchanged 5ms CI guard. No thresholds,
sample counts, skip rules or payload workload have been relaxed.

The Linux accepted-socket target passed both tests locally
(`/tmp/workspacer-tcp-options-check.log`): actual hub/MCP health responses were
observed and their accepted sockets had TCP_NODELAY set, while client sockets
were deliberately unset. The false/true control also passed. This establishes
socket configuration; the corrected optimized timing receipt remains pending.
