# duckers

duckers is a DuckDB extension for drawing charts from SQL with the Rust plotting library
[`plotters`](https://github.com/plotters-rs/plotters).
It is written in Rust against DuckDB's C extension API
and started from [`duckdb/extension-template-rs`](https://github.com/duckdb/extension-template-rs),
so building it needs no DuckDB source tree and no C or C++ code.

## Status

The extension builds and passes its tests, but the only function it registers so far is `duckers_version()`.
`plotters` is not a dependency yet, and the SQL interface for plotting has not been designed.

## Requirements

- A Rust toolchain, `make`, `git` and `python3`
- [uv](https://docs.astral.sh/uv/), optional; when present it builds the Python test environment
- The `duckdb` CLI, for `make shell`

`mise install` installs `uv` and `duckdb` at the versions pinned in `mise.toml`.

## Building and testing

```sh
make               # configure, then debug build into build/debug/duckers.duckdb_extension
make test          # SQLLogicTests in test/sql against the debug build
make shell         # DuckDB shell with the debug build loaded
make release       # optimized build into build/release
make clean_all     # remove build output and configure state
```

The build files come from [`extension-ci-tools`](https://github.com/duckdb/extension-ci-tools).
The first `make` clones that repository into `./extension-ci-tools` at the ref in `CI_TOOLS_REF`.
`make configure` then creates the Python test environment in `configure/venv`:
with uv it runs `uv sync` against `pyproject.toml` and `uv.lock`, and without uv it falls back to pip, as CI does.

Tests are SQLLogicTest files in `test/sql`, run with DuckDB's Python test runner.

## Loading the extension

The build is unsigned, so DuckDB has to be started with `-unsigned`:

```sql
-- duckdb -unsigned
LOAD 'build/debug/duckers.duckdb_extension';
SELECT duckers_version();
```

## DuckDB version

The `duckdb` crate uses DuckDB's unstable C API, so a build loads only in the exact DuckDB version it was built for,
currently v1.5.5.
When moving to another version, update these together:

- the `duckdb` crate version in `Cargo.toml` (`1.10505.x` corresponds to v1.5.5)
- `TARGET_DUCKDB_VERSION` in `Makefile`
- `duckdb` in `pyproject.toml` and `mise.toml`
- `duckdb_version` and `ci_tools_version` in `.github/workflows/MainDistributionPipeline.yml`,
  and `CI_TOOLS_REF` in `Makefile`
