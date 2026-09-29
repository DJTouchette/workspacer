# Optional Go MCP wire oracle

This standard-library-only AST tool captures the original facade's typed input
rules without compiling the Go hub or MCP SDK. It records a rule for each of
100 built-in tools plus SHA-256 digests of the16 original production inputs.

```sh
go run ./tools/go-mcp-wire-reference/main.go --root . --check services/hub-rs/assets/mcp-wire-contract.json
# Explicitly reviewed recapture only:
go run ./tools/go-mcp-wire-reference/main.go --root . > services/hub-rs/assets/mcp-wire-contract.json
```

Rules distinguish omitted zero-valued scalars, nil pointers, empty slices/maps,
embedded struct fields and nested typed array items. Pointer false, zero and
empty strings remain values; an empty pointed-to struct remains present.
Freeform object inputs and strict raw routing-preference inputs retain their
own handling. Unknown/unmapped/recursive shapes fail rather than silently
losing fields. The input-type registry follows actual generic registration
calls and specialized AddTool closure types; schema names alone cannot infer
Go pointer semantics.

This is an optional historical oracle, not a routine generation or build
requirement. `python3 scripts/mcp-catalog.py --write` and `--check` consume the
reviewed JSON contracts and need neither Go nor the legacy source tree.
