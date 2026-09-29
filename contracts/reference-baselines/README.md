# Captured Go reference baselines

The six named fixtures in `manifest.json` retain expected behavior from the
retired Go implementation. They require an actual Rust replay, case floors,
vocabulary validation, and mutation guards. They do not need an artificial
second live implementation after Go removal. Other portable fixtures still
require tests in two languages; an independent TypeScript consumer cannot be
replaced by this exemption.

Each entry records the Git reference commit, source and reference-loader hashes,
exact captured fixture hash, every case-block floor, and Rust replay file/needle.
This capture pins an existing migration contract; it does not claim a new Go
execution. Source hashes are checked while source remains present and preserved
as provenance after removal. Fixture and replay checks always run.

Run `python3 scripts/capture-contract-baselines.py --check` to verify. Refresh is
explicit: `--capture` requires all named reference sources to exist and match the
current committed revision. Changing a baseline after source retirement needs a
reviewed reference update; simply deleting a guard or adding a new exemption is
not a refresh. Rust `corpus_ownership` tests independently verify this manifest
and deliberately corrupt its fields to prove failures remain observable.

Hash inputs are pinned to LF checkout bytes by the repository `.gitattributes`
(contract JSON and retained Go references), including on Windows with
`core.autocrlf=true`. The guard compares original bytes; it does not normalize
whitespace or JSON values before hashing. A Git checkout-filter regression
checks that platform newline conversion cannot silently change those bytes.
