# DuckDB v2 preview builds and C API details

Research date: 2026-09-27.
Sources inspected:

- `duckdb/duckdb` branch `v2.0-cyanoptera`, HEAD `d8a1bd4f` (2026-09-25), the same commit as the earlier reports.
- `duckdb/duckdb` branch `v1.5-variegata`, HEAD `46f1c14caa` (2026-09-25), through the GitHub API.
- `duckdb/duckdb-rs` `main`, HEAD `18c6d31` (2026-09-25).
- `duckdb/extension-ci-tools` `main`, HEAD `20bad04` (2026-09-25).
- `duckdb/extension-template-rs` `main`, HEAD `abb7b2f` (2026-07-23).
- Preview binaries: the nightly CLI `v2.0.0-alpha43546` (source id `d8a1bd4f4f`),
  the staged CLI `v2.0.0-alpha43385` (`ca15f79c32`), and the Python wheel `duckdb==2.0.0.dev2609250715`.
- A throwaway probe extension in C, compiled with `gcc` against `duckdb_extension_v2.h` from the nightly shared-libs
  tarball, and loaded into both preview CLIs and the Python wheel.

This report extends [the C API report](2026-09-27-duckdb-v2-c-api-from-rust.md)
and [the parser report](2026-09-27-duckdb-v2-parser-extensibility.md).
"Verified" means run against a preview binary or read directly in the source.
"Inferred" means concluded from the spec or source without running it.

## Summary

- DuckDB v2 runs today on Linux x86_64, as a nightly CLI tarball, a staged "latest alpha" CLI, a `--pre` Python wheel,
  a Java snapshot, and an R-universe package.
  There is no GitHub pre-release, no npm or Homebrew build.
- A C-API v2 extension loads into the preview with `-unsigned`.
  It still needs the 512-byte metadata footer, with ABI `C_STRUCT` and C API version `v2.0.0`.
  The v1 extension-ci-tools footer script produces a valid v2 footer.
- Aggregates work end to end, including `ANY` parameters, varargs, named parameters with defaults,
  finalize results of 100 MB, and the dot-call syntax.
- `agg(x ORDER BY y)` and `agg(x) OVER ()` crash the process for C-API aggregates, in v1 and v2 alike.
  This is issue [#26109](https://github.com/duckdb/duckdb/issues/26109), open and unfixed on every branch.
  Declaring the aggregate order-independent avoids the crash, and the planner then drops the `ORDER BY`.
- A second aggregate registered under an existing name fails,
  so the v2 C API cannot express aggregate overloads.
- A custom type `CHART` over `BLOB` reports `CHART` from `typeof()`, drives overload resolution,
  and renders through a registered `CHART → VARCHAR` cast in the CLI.
  Casts to and from `BLOB` must be registered explicitly.
- An extension can read settings through a context but cannot register or write one,
  and it cannot run SQL from a callback.
- With `threads = 1`, every callback runs on the thread that issued the query.
  With more threads, that thread does part of the work and background workers do the rest.
- duckdb-rs generates bindings for `duckdb_v2.h` only.
  Nothing in duckdb-rs, extension-template-rs or extension-ci-tools supports building a v2 loadable extension yet.
- `COPY (…) TO 'f' (FORMAT blob)` writes raw bytes in both 1.5.5 and v2, and accepts a `CHART` column.

## Obtaining a v2 build

### Channels

| Channel | What it gives on 2026-09-27 | Status |
| --- | --- | --- |
| GitHub releases/tags on `duckdb/duckdb` | Newest is `v1.5.5` (2026-07-22); no v2 tag or pre-release | Verified |
| `artifacts.duckdb.org/v2.0-cyanoptera/duckdb-cli-linux-amd64.tar.gz` | `v2.0.0-alpha43546`, rebuilt nightly (Last-Modified 2026-09-27 01:13 GMT) | Verified, runs |
| `artifacts.duckdb.org/v2.0-cyanoptera/duckdb-shared-libs-linux-amd64.tar.gz` | `libduckdb.so`, `duckdb_v2.h`, `duckdb_extension_v2.h`, v1 headers | Verified |
| `curl https://install.duckdb.org \| DUCKDB_VERSION=alpha bash` | The staged build named by `duckdb-staging.duckdb.org/latest_alpha_version.txt`, currently `ca15f79c32/v2.0.0-alpha43385` | Verified by downloading the same URL the script uses |
| `pip install duckdb --pre` | `2.0.0.dev2609250715` (reports `v2.0.0-alpha43385`); five `2.0.0.devN` uploads since 2026-09-12 | Verified, runs |
| Java | `org.duckdb:duckdb_jdbc:2.0.0-alpha43385-881` from `duckdb-staging.duckdb.org/duckdb/duckdb-java/maven/` | Listed on the preview page |
| R | `duckdb.2.0.dev` `1.99.99.9000.1801` on `duckdb.r-universe.dev`, built from duckdb-r branch `v2.0-cyanoptera-green`; the preview page's `pak::pak("duckdb/duckdb-r")` installs `1.5.5.9028`, a v1.5 build | Verified via the r-universe API and the DESCRIPTION files |
| Node (`@duckdb/node-api`) | dist-tags `latest` `1.5.5-r.5`, `lts-v1.4`; the preview page says the Neo nightly is unavailable | Verified |
| Homebrew | Formula `duckdb` stable `1.5.5`, no `head` | Verified |
| GitHub Actions artifacts | The `Main` workflow on `v2.0-cyanoptera` uploads `duckdb-cli-*`, `duckdb-shared-libs-*` and extension bundles, kept 90 days; downloading needs a GitHub login | Verified (listed, not downloaded) |

The install script writes to `~/.duckdb/cli/<version>`, repoints `~/.duckdb/cli/latest`,
and symlinks `~/.local/bin/duckdb` if nothing is there yet.
It was not run.

`duckdb/duckdb` `main` is already a separate v2.1 line
(`v2.1.0-alpha43482`, forked from `v2.0-cyanoptera` on 2026-09-14),
per duckdb-rs PR [#866](https://github.com/duckdb/duckdb-rs/pull/866).

### What runs (verified)

```text
$ duckdb -c "SELECT version(), * FROM pragma_version()"
v2.0.0-alpha43546 | v2.0.0-alpha43546 | d8a1bd4f4f | Cyanoptera     (nightly tarball)
v2.0.0-alpha43385 | v2.0.0-alpha43385 | ca15f79c32 | Cyanoptera     (staged CLI and Python wheel)
```

- `SELECT * FROM duckdb_grammar_extensions()` exists and returns zero rows in both CLIs and the wheel.
- Core extensions: `INSTALL inet` works from the staged `alpha43385` CLI and the wheel.
  From the nightly `alpha43546` it fails with HTTP 404 (core) or 403 (`core_nightly`),
  because no extensions are published for that build.
- The same probe binary loads into `alpha43546`, `alpha43385` and the wheel.
  GitHub's compare shows the two builds 65 commits apart with no change under `api_spec/`.

### Unsigned loading and the metadata footer (verified)

- `duckdb -unsigned` sets `allow_unsigned_extensions = true`, as in v1.
  The Python wheel takes `config={'allow_unsigned_extensions': 'true'}`.
- Without it, loading the probe fails with "signature is either missing or invalid".
- A bare shared library without a footer is refused even with `-unsigned`:
  "The metadata at the end of the file is invalid".
  With `allow_extensions_metadata_mismatch = true` as well, the load dies on an internal error ("Unknown ABI type").
- The footer layout is unchanged from v1
  (`src/main/extension/extension_load.cpp:366-406`, `src/include/duckdb/main/extension.hpp:45-67`):
  the last 512 bytes of the file are eight 32-byte NUL-padded fields followed by a 256-byte signature.
  Counting back from the signature, the fields are: magic `"4"`, platform, DuckDB or C API version, extension version,
  ABI type, and three unused.
  The extension-ci-tools script writes a 22-byte WebAssembly custom-section header before the fields,
  which the loader ignores.
- How the loader picks the entry point (`extension_load.cpp:30-41`, `:876-898`; `src/main/extension.cpp:42-99`):
  - ABI `C_STRUCT` with version `v2.x.y` selects the v2 entry point `<name>_init_c_api_v2`.
    The version must be at most the engine's v2 C API version, `v2.0.0` in these builds.
    `v2.1.0` is refused with "we can only load extensions built for DuckDB C API 'v2.0.0' and lower".
  - ABI `C_STRUCT` with `v1.x.y` still selects the v1 entry point `<name>_init_c_api`.
    DuckDB v2 serves the v1 C API up to `v1.5.6` (`src/include/duckdb_extension.h:44-46`).
  - ABI `C_STRUCT_UNSTABLE` always means v2 and requires the exact engine version string, e.g. `v2.0.0-alpha43546`.
    Today's extension-template-rs builds v1 extensions with `USE_UNSTABLE_C_API=1`, which produces this ABI tag,
    so those binaries cannot load on v2 without a rebuild.
  - The platform field must match (`linux_amd64` here).
- `INSTALL '<path>.duckdb_extension'` of an unsigned file also needs `-unsigned`,
  and then installs into `<extension_directory>/v2.0.0-alpha43546/linux_amd64/`.
- The load path has no v2-specific signing step (source).
  The 256-byte signature is checked against the core and community keys as in v1,
  and skipped when unsigned extensions are allowed.
- Community extensions build a v2 leg from `repo.ref_next` but do not deploy it
  ([community-extensions#2723](https://github.com/duckdb/community-extensions/issues/2723), open).

## Aggregates in the v2 C API

### Lifecycle (from `api_spec/v2/function/aggregate.yaml`, verified by the probe)

1. Create with `aggregate_function_create_with_extension(extension, &f)`,
   configure with `aggregate_function_set_name` and the signature from `aggregate_function_get_signature`,
   and publish with `aggregate_function_register`.
   Registration requires a name plus the size, init, update, combine and finalize callbacks.
   An `ANY` return type is accepted only with a bind callback
   that sets the concrete type (`src/main/capi/v2/capi_v2_func_aggregate.cpp:314-369`).
2. **bind** (optional) is the only aggregate callback that receives a `context`.
   It can read argument types, fold constant arguments to values, set the return type, and attach bind data
   (an opaque pointer with destructor and equality callbacks).
3. **size** reports the byte size of one state and may depend on the bind data.
   DuckDB allocates the state memory itself.
4. **init** initialises an array of uninitialised states in place.
5. **update** receives the row count, one vector per argument, and one state pointer per input row.
   Several rows may point to the same state.
6. **combine** merges source state *i* into target state *i* and must not modify the source.
7. **finalize** writes state *i* into row `offset + i` of a result vector.
8. **destroy** (optional) releases state resources.
   The YAML says it runs on states "discarded without being finalized",
   but the probe counted equal numbers of init and destroy calls in ungrouped, grouped and window queries,
   finalized states included.

Every callback reports errors through its error slot.
Input vectors are not flattened before update or scalar exec: `range()` delivers `SEQUENCE` vectors,
which `vector_get_view` rejects until `vector_flatten` is called.
The v1 C API flattened aggregate inputs before calling update.

### Signatures (verified)

- `ANY` parameters work; bind sees the concrete argument type (`BIGINT`, `DECIMAL(2,1)`, `INTEGER[]`, `STRUCT(…)`).
- `function_signature_set_varargs(ANY)` gives a heterogeneous variadic tail, including zero arguments.
- `function_signature_add_parameter` takes a name and an optional default value.
  For `probe_sig(x ANY, sep VARCHAR := ',')`, all of `probe_sig(i)`, `probe_sig(i, '|')`,
  `probe_sig(i, sep := '|')` and `probe_sig(x := i, sep := '|')` bind, and the default reaches bind as a constant.
  An unknown name is a binder error listing the candidates.
- `aggregate_function_bind_get_arg_value` folds `getvariable('sep')` to its constant.
- Finalize wrote a 100,000,000-byte `BLOB` per group through `vector_get_arena` and `arena_allocate`.
  The inferred ceiling is 4 GiB − 1 per value, since `duckdb_v2_bytes.length` is a `uint32_t`.

### Overloads (verified)

- There is no aggregate function-set handle in the v2 C API.
- Registering a second aggregate under an existing name fails with "Not implemented Error:
  GetAlterInfo not implemented for this type".
  Registration uses `ALTER_ON_CONFLICT` (`src/main/extension/extension_loader.cpp:154-159`),
  which calls `CreateInfo::GetAlterInfo` (`src/catalog/catalog_entry/duck_schema_entry.cpp:220-229`),
  and only the scalar and table function infos override it.
- Scalar overloads registered the same way merge as expected: three `probe_kind` overloads coexist.
- Workarounds: one signature with `ANY` or varargs plus validation in bind, or distinct names.

### ORDER BY, windows and issue #26109

- The spec exposes the order and DISTINCT sensitivity as properties (`api_spec/v2/function/properties.yaml`):
  `FUNCTION_PROPERTY_AGG_ORDER_DEPENDENT` and `FUNCTION_PROPERTY_AGG_DISTINCT_DEPENDENT`, both defaulting to `YES`.
  There is no other notion of an "ordered aggregate".
- Verified on `v2.0.0-alpha43546` with a sum aggregate written to the documented contract:
  - `probe_sum(i ORDER BY i DESC)` segfaults, even on three rows, grouped or not.
    gdb shows `sum_update ← CV2AggregateUpdate ← SortedAggregateFunction::Finalize` on the main thread.
  - `probe_sum(i) OVER ()` segfaults.
  - `probe_sum(i) OVER (ORDER BY i ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)` and `probe_sum(DISTINCT …)` work.
  - With `AGG_ORDER_DEPENDENT_NO`, `probe_sum_oi(i ORDER BY i DESC)` returns the right sum, and `EXPLAIN` shows the
    `ORDER BY` removed.
- Verified on `libduckdb` v1.5.5 with the reproducer from the issue: plain aggregate fine,
  ordered and `OVER ()` both exit with signal 11.
- Cause (source, verified): for these two callers the executor passes a constant state vector with `count > 1`.
  `CV2AggregateUpdate` reads it as flat without flattening (`capi_v2_func_aggregate.cpp:232`),
  and v2 C-API aggregates register no cluster-update callback,
  so `SortedAggregateFunction` falls back to the plain update
  (`src/function/aggregate/sorted_aggregate_function.cpp:468-478`).
  The callback then reads `states[1..count)` past the end of a one-element buffer, and it cannot detect this.
- Status: issue #26109 is open, labelled `reproduced`, with no comments and no linked PR.
  The v2 branch at `d8a1bd4f` and `v1.5-variegata` at `46f1c14caa` (`src/main/capi/aggregate_function-c.cpp:102`) still
  have the unflattened read.
  The last v1.5 release, v1.5.5, predates the report.
- Inferred consequence for a chart aggregate: declare it order-independent and sort inside finalize,
  and document that `OVER ()` crashes until the fix lands.

## Custom types (verified)

Probe: `CHART` registered with `custom_type_create_with_extension`, base `BLOB`,
then fetched as a logical type with `context_create_type_from_text(ctx, "CHART")` on the entrypoint context,
which succeeds right after registration.
A `CHART → VARCHAR` cast is registered with `cast_function_create_with_extension`.

- `typeof(probe_chart('hello'))` is `CHART`; the CLI column header shows `chart`.
- Overloads `probe_kind(CHART)`, `probe_kind(BLOB)` and `probe_kind(VARCHAR)` dispatch correctly.
- The CLI box renderer, `-csv` and `-json` all print the registered cast (`<CHART 5 bytes>`), not the bytes.
  The Python wheel returns the raw `bytes`.
- A second custom type without a cast renders through the base type's text form (`\x00abc`).
- There is no implicit or explicit cast between a custom type and its base:
  `CHART → BLOB` and `BLOB → CHART` both fail with "Unimplemented type for cast" until registered.
  A string literal casts directly (`'a'::CHART`).
- `CREATE TABLE t(c CHART)` and inserts work.
- Implementation: the custom type is the base type tagged with an alias
  (`src/main/capi/v2/capi_v2_custom_type.cpp:10-67`), so its `LogicalTypeId` stays `BLOB`.
  This is why `COPY … (FORMAT blob)` accepts it (see below).
- `cast_function_set_implicit_cast_cost` exists for making a registered cast implicit (spec only, not tried).

## Dot-call syntax (verified)

- The v2 PEG grammar keeps it: `BaseExpression <- SingleExpression IndirectionList?`, with
  `DotMethodOperator <- '.' MethodExpression` and
  `MethodExpressionArgumentList <- DistinctOrAll? MethodFunctionArguments? OrderByClause? IgnoreOrRespectNulls?`
  (`src/parser/peg/grammar/statements/expression.gram:280-291`).
- The transformer inserts the receiver as the first argument of the resulting function expression
  (`src/parser/peg/transformer/transform_expression.cpp:114`).
- It works for aggregates and extension functions:
  `i.probe_sum()`, `(i + 1).probe_sum()`, `i.probe_sig(sep := '|')`, `i.string_agg(',' ORDER BY i DESC)`,
  `i.list(ORDER BY i DESC)`.
- v2 also accepts a bare string literal as receiver (`'hello'.probe_chart()`); v1.5.5 needs parentheses.
- v1.5.5 supports aggregates too (`i.sum()`, `i.string_agg(',')`),
  although the [docs](https://duckdb.org/docs/current/sql/functions/overview#function-chaining-via-the-dot-operator) say
  chaining is "limited to scalar functions".
  The PEG parser documentation page does not mention dot calls.

## Settings, options and variables

Verified:

- `context_get_option_by_name(ctx, name, &opt)` plus `option_get_setting` reads any option from a scalar exec callback:
  `threads` gave `12`, `memory_limit` gave `24.8 GiB`.
  An unknown name returns "unknown configuration option".
- `SET duckers_default_size = 3` fails as an unrecognized parameter.
- `getvariable('x')` passed as an aggregate argument arrives in bind as a folded constant.

From the spec (`api_spec/v2/configuration/configuration.yaml`, `connection/connection.yaml`, `instance/instance.yaml`):

- Options are written only through `instance_set_option` and `connection_set_option`.
  "A context is a read scope".
- Nothing registers a new option, and nothing reads `SET VARIABLE` values directly.
- A context reaches scalar bind, init and exec, aggregate bind, table, cast, copy and replacement-scan callbacks.
  Aggregate update, combine and finalize get no context, so a setting has to be read in bind and carried in bind data.

Inferred:

- The C++ grammar shim could register `duckers_*` options through the C++ API,
  and the Rust core could read them with `context_get_option_by_name`,
  since option listings include extension-registered options.
- Named parameters with defaults are the C-API-only way to make chart options optional.

## Running SQL from inside a function

Definitive for `d8a1bd4f`, from all 557 functions and 35 callbacks in `api_spec/v2/**/*.yaml`:

- Every execution entry point takes a `connection`:
  `statement_execute` (`query_result/query_result.yaml`), `prepared_statement_create` (`prepared_statement/`),
  `parse_sql` and `statement_bind` (`sql_statement/sql_statement.yaml`).
- The only producer of a `connection` is `connection_create(instance)`.
  The only producer of an `instance` is `instance_create(environment)`, which opens a new database.
  No function maps a `context` or an `extension` to an `instance` or `connection`.
- Functions taking a `context` only read options, create types, values, chunks and collections, open the file system,
  log, or build Arrow importers and exporters (`common/common.yaml`: a context is "not for registration").
- The entrypoint field description in `extension/extension.yaml` calls the context one "to read and run queries
  through", but no API consumes it for queries.
- The closest mechanism is a replacement scan claiming an unknown table name with `replacement_scan_set_subquery`,
  one `SELECT` parsed at the call, whose callback context "may be used to read settings, but not to run queries"
  (`function/replacement_scan.yaml`).

## Threading (verified)

- `TaskScheduler::SetThreads(total, external)` starts `total − external` background threads
  (`src/parallel/task_scheduler.cpp:307-321`).
  `external_threads` defaults to 1 (`src/common/settings.json:937-941`).
- The thread that issued the query executes pipeline tasks itself, in
  `ClientContext::ExecuteTaskInternal` (`src/main/client_context.cpp:750-760`) → `Executor::ExecuteTask`
  (`src/parallel/executor.cpp:452`).
- The probe checked `gettid() == getpid()` in each callback:
  - Entrypoint and aggregate bind: main thread.
  - `threads = 12`, 30M-row table: scalar exec rows 2,597,760 on main and 27,402,240 on workers.
    Aggregate update calls: 2,100 on main and 12,549 on workers.
    Grouped aggregates also finalized on workers.
  - `threads = 1`: every callback on the main thread.
  - A `range()` source ran entirely on the main thread even with 12 threads.
- Inferred: only `threads = 1` guarantees that callbacks run on the calling thread.
  In the CLI that is the process main thread; in Python it is whichever thread called `execute`.
  A window that must live on the process main thread needs its own arrangement.

## Rust bindings and tooling

- duckdb-rs `libduckdb-sys` feature `capi-v2` runs bindgen over `wrapper_v2.h`
  (`#include "duckdb/duckdb_v2.h"`) into `libduckdb_sys::v2`.
  The bindings contain the `duckdb_v2_extension_input` struct but no `duckdb_ext_api_v2` table, no `get_api` wrapper,
  and no `_init_c_api_v2` entry point.
  `duckdb-loadable-macros` still generates only the v1 `<name>_init_c_api`.
- `duckdb-neo` calls the linked `duckdb_v2_*` symbols directly and offers `Extension::from_raw(handle)`.
  The preview CLI exports no `duckdb_*` symbols dynamically
  (`nm -D`: zero),
  so an extension built on duckdb-neo would have to bundle its own `libduckdb` instead of using the host's table
  (inferred: unusable for a loadable extension as it stands).
- Neither `duckdb-neo` nor `libduckdb-sys 1.20000.0` is on crates.io, which still tops out at `1.10505.0`.
  `main` pins `v2.0.0-alpha43089` (`crates/libduckdb-sys/.duckdb-release`).
  PR [#861](https://github.com/duckdb/duckdb-rs/pull/861)
  (merged 2026-09-24) added function registration, saying "don't start building against this yet".
  Open: [#866](https://github.com/duckdb/duckdb-rs/pull/866)
  (bump to `v2.1.0-alpha43482`),
  [#841](https://github.com/duckdb/duckdb-rs/issues/841)
  (v2 API feedback, 7 comments), [#875](https://github.com/duckdb/duckdb-rs/issues/875) (C++ API).
- extension-template-rs has only `main`, targeting v1.5.5 with `USE_UNSTABLE_C_API=1`.
  It has no v2 branch, issue or PR.
- extension-ci-tools has release branches up to `v1.5.5` and no v2 branch.
  `main` has no v2 logic in `makefiles/c_api_extensions/`.
  Its `append_extension_metadata.py -p linux_amd64 -dv v2.0.0`, with the default `--abi-type C_STRUCT`,
  produced a probe binary that loads on the preview (verified).
  Wasm linking exports only `_<name>_init_c_api`.
- DuckDB ships an in-tree C++ wrapper over the v2 C API for extensions (`tools/cpp/duckdb_cpp_extension.hpp`,
  demo `test/extension/cpp_api_demo.cpp`).

## COPY … (FORMAT blob) (verified)

- 1.5.5 and both v2 builds write exactly the bytes: `'abc'::BLOB` produced a 3-byte file `61 62 63`.
- Several rows are concatenated into one file (`abc` + `def` → 6 bytes).
- The source must be a single `BLOB` column; `VARCHAR` or two columns fail with
  `"COPY (FORMAT BLOB)" only supports a single BLOB column` (`src/function/copy_blob.cpp:36-40`).
- A `CHART` value passes the check and writes its raw bytes, because the check compares `LogicalTypeId::BLOB`.
- A 1,000,000-byte value from an extension aggregate copied out intact.

## Sources

- [DuckDB preview installation page](https://duckdb.org/install/preview.html)
- [A Preview of DuckDB v2.0](https://duckdb.org/2026/08/17/duckdb-20-highlights)
- Nightly artifacts: `https://artifacts.duckdb.org/v2.0-cyanoptera/duckdb-cli-linux-amd64.tar.gz`,
  `https://artifacts.duckdb.org/v2.0-cyanoptera/duckdb-shared-libs-linux-amd64.tar.gz`
- Staged alpha: `https://duckdb-staging.duckdb.org/latest_alpha_version.txt`,
  `https://duckdb-staging.duckdb.org/ca15f79c32/v2.0.0-alpha43385/duckdb/duckdb/github_release/duckdb-cli-linux-amd64.tar.gz`
- [install.duckdb.org](https://install.duckdb.org) (script revision `50350db8c3`)
- [PyPI duckdb release history](https://pypi.org/project/duckdb/#history)
- [duckdb.r-universe.dev](https://duckdb.r-universe.dev)
- [duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109)
- [duckdb-rs #841](https://github.com/duckdb/duckdb-rs/issues/841),
  [#855](https://github.com/duckdb/duckdb-rs/pull/855), [#861](https://github.com/duckdb/duckdb-rs/pull/861),
  [#866](https://github.com/duckdb/duckdb-rs/pull/866), [#875](https://github.com/duckdb/duckdb-rs/issues/875)
- [community-extensions #2723](https://github.com/duckdb/community-extensions/issues/2723),
  [#2765](https://github.com/duckdb/community-extensions/pull/2765)
- [Function chaining via the dot operator](https://duckdb.org/docs/current/sql/functions/overview#function-chaining-via-the-dot-operator),
  [PEG parser](https://duckdb.org/docs/current/sql/peg_parser)
- DuckDB source (`v2.0-cyanoptera` `d8a1bd4f`):
  `api_spec/v2/function/{aggregate,signature,properties,cast,scalar,replacement_scan}.yaml`,
  `api_spec/v2/logical_type/{custom_type,logical_type}.yaml`, `api_spec/v2/configuration/configuration.yaml`,
  `api_spec/v2/{common/common,extension/extension,connection/connection,instance/instance}.yaml`,
  `src/include/duckdb_extension_v2.h`, `src/main/capi/v2/capi_v2_func_aggregate.cpp`,
  `src/main/capi/v2/capi_v2_custom_type.cpp`, `src/main/extension/extension_load.cpp`, `src/main/extension.cpp`,
  `src/main/extension/extension_loader.cpp`, `src/catalog/catalog_entry/duck_schema_entry.cpp`,
  `src/function/aggregate/sorted_aggregate_function.cpp`, `src/function/copy_blob.cpp`,
  `src/parser/peg/grammar/statements/expression.gram`, `src/parser/peg/transformer/transform_expression.cpp`,
  `src/parallel/task_scheduler.cpp`, `src/parallel/executor.cpp`, `src/common/settings.json`
- duckdb-rs (`18c6d31`): `crates/libduckdb-sys/{Cargo.toml,build.rs,wrapper_v2.h,.duckdb-release}`,
  `crates/duckdb-neo/`, `crates/duckdb-loadable-macros/src/lib.rs`
- extension-ci-tools (`20bad04`): `scripts/append_extension_metadata.py`, `makefiles/c_api_extensions/base.Makefile`
- extension-template-rs (`abb7b2f`): `Makefile`
