# Optional Go regexp reference capture

This standard-library-only tool captures the former jobs guard engine for Rust
compatibility tests. It is **not** invoked by production builds, Rust tests, CI,
or packaged applications. Use the Go version matching the reference being
reviewed; the output records `runtime.Version()` and `unicode.Version`.

From the repository root:

```sh
go run tools/go-regex-reference/main.go cases services/hub-rs/src/services/jobs/go_regex_cases.json > /tmp/job-regex-cases.json
go run tools/go-regex-reference/main.go names > /tmp/job-regex-names.json
diff -u services/hub-rs/src/services/jobs/go_regex_cases.json /tmp/job-regex-cases.json
diff -u services/hub-rs/src/services/jobs/go_regex_names.json /tmp/job-regex-names.json
```

`cases` recomputes match results using Go's `regexp.Compile`/`MatchString` and
refuses a fixture marked invalid if Go accepts it. It never consults Rust output.
`names` considers category/script names, optional category aliases from the Go
installation's own source, and special groups. **Every emitted name must compile
through the actual Go regexp engine.** The probes independently pin `Cn`, `LC`,
alias normalization and rejected property-assignment/binary-property extensions.
This avoids assuming that Unicode table keys and regexp's accepted namespace
are identical.

The checked-in captures use Go **1.25.14**, Unicode **15.0.0**. They contain 63
pattern cases plus the accepted property-name inventory. Review changes before
replacing either fixture, especially when upgrading the reference toolchain.

The Rust adapter preserves ASCII Perl classes, ASCII word boundaries, quoting,
class contexts and repetition limits while retaining Unicode literals/dot and
properties. Property membership and case folding still use the Rust regex
library's Unicode tables, which may be newer than Unicode 15.0. The namespace
capture does not prove equality of every Unicode codepoint across table versions.
