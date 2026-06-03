//go:build race

package bus

// raceEnabled is true when the binary is built with -race. The latency
// distribution test asserts a localhost p99 budget that race instrumentation
// (~10x slowdown) makes meaningless, so it self-skips when this is set.
const raceEnabled = true
