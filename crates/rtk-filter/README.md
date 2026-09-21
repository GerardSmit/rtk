# RTK filters as a Rust library

`rtk-filter` calls RTK's existing output filters in process. It does not execute
commands, read CLI configuration, write recovery files, track usage, or link SQLite.
Its only direct dependencies are `regex`, `serde`, and `serde_json`.

From this fork, select the package explicitly (pin a reviewed commit in production):

```toml
[dependencies]
rtk-filter = { git = "https://github.com/GerardSmit/rtk", branch = "codex/embedded-filters", package = "rtk-filter" }
```

For a checkout/submodule at `external/rtk`:

```toml
[dependencies]
rtk-filter = { path = "external/rtk/crates/rtk-filter" }
```

```rust
use std::borrow::Cow;

let raw = "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
let result = rtk_filter::filter("cargo-test", raw).unwrap_or(Cow::Borrowed(raw));
println!("{result}");
```

`supports_filter(name)` checks availability. `filter(name, text)` returns `None`
for an unknown name, otherwise a borrowed original or an owned compact result.
The crate also exposes RTK's `strip_ansi`, `truncate`, and `join_with_overflow` helpers.

## Input contract

The host executes the command, strips terminal controls where needed, chooses a
matching filter, and retains the raw output for recovery. Filters are lossy.
Selecting a valid filter for the wrong format can discard useful information;
the size guard is not a semantic correctness check. No automatic command rewriting
or filter selection is performed by the public API.

Available names match RTK's pipe filter registry:

| Family | Names | Format notes |
| --- | --- | --- |
| Git | `git-status`, `git-log`, `git-diff` | Status expects porcelain; log/diff expect their text formats. |
| Rust | `cargo-test`, `cargo` | Cargo test text; `cargo` is an alias, not a generic Cargo filter. |
| Go | `go-test`, `go-build` | Tests expect `go test -json`; build expects compiler text. |
| Python | `pytest`, `mypy`, `ruff-check`, `ruff-format` | Ruff check expects JSON diagnostics. |
| JavaScript | `tsc`, `vitest`, `prettier` | Vitest accepts JSON and recognized text output. |
| PHP | `phpunit`, `pest`, `paratest`, `php-test`, `ecs`, `phpstan`, `pint` | Pint expects JSON; PHPStan accepts JSON or text. |
| Other | `ctest`, `grep`, `rg`, `find`, `fd`, `log`, `sqlfluff-lint` | Search expects `file:line:content`; SQLFluff expects JSON. |

Inputs larger than `MAX_INPUT_BYTES` (1 MiB) are returned borrowed without running
a filter. Empty results, results that are not smaller, and unwinding filter panics
also fall back to raw text. Panic recovery does not work with `panic = "abort"`;
Rust's panic hook still runs before an unwind is caught. There is no timeout or
subprocess isolation.

There is no output/history cache. Returned owned output is smaller than the input;
filters may allocate multiple temporary copies and indexes, so 1 MiB is an input
limit, not a peak-memory guarantee. Regex caches are process-wide and independent
of command history. Preserve the original text in the host; CLI recovery hints are
disabled in this build. The parser's passthrough limit uses the CLI default of
2,000 characters without reading any user configuration.

## Development

```sh
cargo test --manifest-path crates/rtk-filter/Cargo.toml
cargo clippy --manifest-path crates/rtk-filter/Cargo.toml --all-targets -- -D warnings
cargo check --all-targets
```

The standalone manifest intentionally does not share a workspace with the CLI:
its dependency resolution never includes the CLI's SQLite version. Both targets
compile the same filter implementations in `src/`. The library build script sets
`rtk_library` only for this crate; CLI-only imports, execution functions, and tests
are excluded with `cfg`. Keep new filter helpers free of CLI side effects and cover
their public behavior in this crate's tests. The library compiles the shared pipe
registry tests as well. This source layout supports Git/path dependencies, not
standalone crates.io packaging (`publish = false`).
