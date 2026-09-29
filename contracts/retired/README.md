# Retired implementation contracts

`path-parameter-shapes.json` preserves the 17 parameter-decoding cases from the
anonymous Go capability harness. The case objects and block specification are
unchanged. The retained Go reference test reads this archived file explicitly.

These were not an active authenticated bus policy at the Rust cutover:
`services/hub/internal/bus/bus.go`'s `conn.authorize` returns immediately for
trusted connections, scoped credentials, and enabled plugin identities. Only
an anonymous `capGrant` unit fixture reaches `paramString`; `handleBus` never
constructs that identity. No TypeScript runtime or test consumes this block.
Retiring the unreachable harness must not introduce new restrictions into the
replacement's live path handlers.

The active `path-containment-cases.json` still pins absolute component-wise path
resolution and selected-object containment, session filenames, library item
directories, project directory names, and spawn cwd normalization. Its Rust and
TypeScript loaders remain part of the active corpus vocabulary and loader guards.
