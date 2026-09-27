# Plan: VISUALIZE for DuckDB v2, rendered with plotters

A ggsql-like `VISUALIZE` clause for DuckDB v2, rendering charts with Rust's `plotters`, split into two extensions.

## Why split

- The v2 C API gives a stable ABI across DuckDB versions, but has no way to extend the grammar
  ([C API report](../docs/reports/2026-09-27-duckdb-v2-c-api-from-rust.md)).
- Grammar extensions (`GrammarExtension` / `GrammarChange`) exist only in the C++ API, which is version-locked and
  still changing before GA ([parser report](../docs/reports/2026-09-27-duckdb-v2-parser-extensibility.md)).

Keeping the syntax in a thin C++ shim confines the version-locked part to a few hundred lines.

## The two extensions

1. **Core (Rust, stable C ABI v2).**
   Our own `-sys` layer over `duckdb_extension_v2.h`, the chart-spec model, and `plotters` rendering.
   Exposes an aggregate such as `viz(spec, x, y, …)` returning a chart value, usable without any custom syntax.
   It is an aggregate because the C API has no table-input table functions
   and no evident way to run nested queries from a callback
   ([C API report, "Gaps observed"](../docs/reports/2026-09-27-duckdb-v2-c-api-from-rust.md#surface-relevant-to-this-project)).
2. **Grammar shim (C++, rebuilt per DuckDB version).**
   Adds `VISUALIZE`/`VISUALISE` via PEG rules,
   and desugars `<query> VISUALIZE …` into `SELECT viz('<spec>', …) FROM (<query>)`.
   It has no rendering logic.
   It is modelled on DuckDB's `loadable_grammar_extension_demo`
   ([parser report, "Reference example"](../docs/reports/2026-09-27-duckdb-v2-parser-extensibility.md#reference-example)).

## Order of work

1. Spike both halves against the v2 preview: a minimal Rust C-ABI extension, and the grammar demo rewired to call it.
2. Core without syntax: `-sys` crate, `viz` aggregate, basic geoms, SVG output.
3. Shim: the `VISUALIZE` grammar and desugaring.
4. Breadth, then distribution (core built once per platform, shim per DuckDB version).

## Open questions

- How the shim is enabled (`active_grammar_extensions` is per-connection) and how it ensures the core is loaded.
- How far the grammar-extension API moves before v2.0 GA.
- Whether layered plots fit in one aggregate call or need one per layer.
