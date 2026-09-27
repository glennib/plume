# duckers

A DuckDB extension that brings the Rust plotting library [`plotters`](https://github.com/plotters-rs/plotters) to SQL.

It is written in pure Rust against DuckDB's C extension API,
based on [`duckdb/extension-template-rs`](https://github.com/duckdb/extension-template-rs).

## Requirements

- A Rust toolchain, `make`, `git` and `python3`
- [uv](https://docs.astral.sh/uv/) (optional, used for the Python test environment when present)
- The `duckdb` CLI for `make shell`

`mise install` provides `uv` and `duckdb` at the versions in `mise.toml`.

## Building and testing

```sh
make               # configure, then debug build into build/debug/duckers.duckdb_extension
make test          # SQLLogicTests in test/sql against the debug build
make shell         # DuckDB shell with the debug build loaded
make release       # optimized build into build/release
make clean_all     # remove build output and configure state
```

The first `make` clones [`extension-ci-tools`](https://github.com/duckdb/extension-ci-tools) into `./extension-ci-tools`
at `CI_TOOLS_REF`.
`make configure` creates the Python test environment in `configure/venv`,
with `uv sync` from `pyproject.toml` and `uv.lock` when uv is available, and with pip otherwise (as in CI).

To load a build manually:

```sql
-- duckdb -unsigned
LOAD 'build/debug/duckers.duckdb_extension';
SELECT duckers_version();
```

## DuckDB version

`duckdb-rs` uses the unstable C API, so a build loads only on the exact DuckDB version it targets, currently v1.5.5.
To move to a new version, update all of these together:

- `duckdb` crate version in `Cargo.toml` (`1.10505.x` is v1.5.5)
- `TARGET_DUCKDB_VERSION` in `Makefile`
- `duckdb` in `pyproject.toml` and `mise.toml`
- `duckdb_version` and `ci_tools_version` in `.github/workflows/MainDistributionPipeline.yml`,
  plus `CI_TOOLS_REF` in `Makefile`
