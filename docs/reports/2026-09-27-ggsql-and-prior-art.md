# ggsql, the-stats-duck and prior art for charts from SQL

Research date: 2026-09-27.
Sources inspected:

- `posit-dev/ggsql` main, HEAD `42fb941f` (2026-09-24); latest release 0.5.2 (2026-09-11).
- `posit-dev/ggsql-duckdb` main, HEAD `49627395` (2026-06-22, ggsql 0.4.1, DuckDB v1.5.4),
  and draft PR #17, head `34709d2c` (2026-09-26).
- `posit-dev/ggsql-python` HEAD `2fffdd60` (0.3.3) and `posit-dev/ggsql-r` HEAD `8b7b4fb7` (0.3.3.9000).
- `KoliStat/the-stats-duck` main, HEAD `efdd24de` (2026-08-06, v0.8.0).
- `duckdb/community-extensions` HEAD `80bed7b1` (2026-09-25), 351 extension descriptors.
- DuckDB source at tag `v1.5.5` and branch `v2.0-cyanoptera` HEAD `d8a1bd4f` (read through the GitHub API).
- DuckDB CLI v1.5.5 (`d8cdaa33fd`) and the `duckdb` Python package 1.5.5, run locally.
  Community extensions were installed into a scratch `extension_directory` for the local checks.

"Verified" below means read in source or reproduced locally.
Inferences are marked as such, and collected again at the end.

## Summary

ggsql is a standalone Rust tool, not a DuckDB extension.
Its `VISUALISE`/`VISUALIZE` part is a **suffix** after an ordinary SQL query, followed by clauses in any order: `DRAW`,
`PLACE`, `SCALE`, `FACET`, `PROJECT` and `LABEL`.
There is no `THEME`, `GUIDE` or `+` operator; layers are repeated `DRAW`/`PLACE` clauses.
It parses the whole query (SQL and visual part) with its own tree-sitter grammar,
runs the SQL through a pluggable reader, and renders with a writer: Vega-Lite JSON by default, or SVG, PDF, `.hep`,
and GPU-backed PNG/JPEG/TIFF/WebP.
There is no plotnine writer.

The DuckDB integration lives in a separate repository, `posit-dev/ggsql-duckdb` (community extension `ggsql`, MIT).
It uses the legacy `ParserExtension` fallback, re-runs the SQL on a sibling connection,
and by default shows the chart **in the browser** through an in-process HTTP server, returning no rows.
A setting switches it to returning a URL, the Vega-Lite spec, or a self-contained HTML page as a `VARCHAR`.
Nothing is drawn in the terminal.
The only DuckDB v2 work is a draft PR by an outside contributor that keeps the legacy fallback hook.

the-stats-duck's `VISUALIZE` is a smaller, **statement-initial** dialect
(`VISUALIZE … FROM <table> DRAW …`) that returns one row of Vega-Lite v5 spec plus per-layer SQL for the client to run.
Its issue #46 plans a move to PEG grammar hooks on v2; no work on it has started.

The DuckDB CLI can carry a chart to the user as text, through a pipe or temp file handed to an external program,
or as a file written with `COPY … (FORMAT blob)`.
Its default `duckbox` mode escapes control characters,
so newlines and terminal-graphics escape sequences only survive in modes such as `list` and `line`.
Every CLI output mode renders a value by casting it to `VARCHAR` through the database's cast set
(`json` mode goes through `JSON` first for nested types),
so an extension controls how its custom type is displayed by registering a cast to `VARCHAR`.
This holds in v1.5.5 and on the v2 branch.

## ggsql

### Project layout and license

- `posit-dev/ggsql` (MIT, "Copyright 2025 ggsql authors") is a Cargo workspace:
  the core crate `ggsql` (`src/`), the `ggsql` CLI, a tree-sitter grammar, a Jupyter kernel, WebAssembly bindings,
  and a VS Code/Positron extension.
- Bindings live in separate repositories:
  `posit-dev/ggsql-python` (PyPI `ggsql` 0.3.3, MIT in `pyproject.toml`),
  `posit-dev/ggsql-r` (CRAN `ggsql`, "MIT + file LICENSE"),
  and `posit-dev/ggsql-duckdb` (MIT).
- The project calls itself alpha ("main architectural parts finished, but is still subject to change", `README.md`).

### Clause grammar

The docs state that apart from `VISUALISE` coming first, "the order of these clauses is arbitrary"
(`doc/syntax/index.qmd`).
The grammar below is condensed from `tree-sitter-ggsql/grammar.js` (rules `query` through `label_clause`).
It is **not verbatim**; keywords are case-insensitive.

```text
query        := [sql_portion] visualise_stmt*
visualise_stmt := (VISUALISE | VISUALIZE) [mapping, ...] [FROM source] clause*
clause       := draw | place | scale | facet | project | label
draw         := DRAW geom
                  [MAPPING (mapping, ... [FROM source] | FROM source)]
                  [REMAPPING mapping, ...]
                  [SETTING name => value, ...]
                  [FILTER <raw SQL condition>]
                  [PARTITION BY column, ...]
                  [ORDER BY <raw SQL order list>]
place        := PLACE geom [SETTING name => value, ...]
scale        := SCALE [CONTINUOUS | DISCRETE | BINNED | ORDINAL | IDENTITY] aesthetic
                  [FROM array] [TO (array | palette)] [VIA transform]
                  [SETTING name => value, ...]
                  [RENAMING (value | * | NULL) => (string | NULL), ...]
facet        := FACET column, ... [BY column, ...] [SETTING name => value, ...]
project      := PROJECT [aesthetic, ...] TO coord [SETTING name => value, ...]
label        := LABEL [name => (string | NULL), ...]
mapping      := * | (column | literal) AS aesthetic | column
source       := identifier | 'file path' | namespace:name | {{ jinja }}
value        := string | number | boolean | NULL | Inf | array
array        := [elem, ...] | (elem, ...)
geom         := point | line | path | bar | area | tile | polygon | ribbon | histogram | density
              | smooth | boxplot | violin | text | label | segment | arrow | rule | range | spatial
```

Semantics, from `doc/syntax/clause/*.qmd`:

- Mappings in `VISUALISE` are global and inherited by layers; `MAPPING` in a layer overrides them,
  and `null AS <aes>` blocks inheritance.
- `SETTING` sets literal aesthetics (bypassing scales) or layer parameters such as `bins`, `position` and `aggregate`.
- `REMAPPING` maps columns computed by a layer's statistic (e.g. `density AS y`) to aesthetics.
- `FILTER` is passed into a `WHERE` clause for the layer's data; `ORDER BY` fixes row order, e.g. for `path`.
- `PLACE` is an annotation layer with literal values only, which may be arrays.
- `SCALE` picks the scale type (inferred when omitted), input range, output range or palette, transform and breaks;
  `RENAMING * => '{:Title}'` formats break labels.
- `PROJECT` picks the coordinate system (`cartesian`, `polar`, `crs`); `PROJECT y, x TO cartesian` flips axes.
- `LABEL` sets `title`, `subtitle`, `caption` and axis/legend titles.

### Position relative to the query

- Everything before `VISUALISE` is ordinary SQL sent to the backend,
  possibly several `;`-separated statements (`sql_portion` in the grammar).
  "if it ends with a `SELECT` query this will automatically be added as the global data for the plot"
  (`doc/syntax/clause/visualise.qmd`).
- CTEs in the pre-query are materialised as temporary tables (`src/execute/cte.rs`),
  so `VISUALISE … FROM cte` and `MAPPING … FROM cte` can refer to them.
- `VISUALISE … FROM <source>` stands in for a missing trailing `SELECT`; the parser injects `SELECT * FROM <source>`.
- Non-`SELECT` statements (`INSTALL`, `SET`, `CREATE`, `INSERT`, …) run first, as setup.
- The trailing `SELECT` is materialised as a temporary table named `__ggsql_global_<uuid>__`.
  The name appeared in a local error message; `Reader::materialize_table` documents `CREATE TEMP TABLE … AS …`.
  A trailing SQL `ORDER BY` is accepted before `VISUALISE` (verified locally).
  In a 5-row local test its order survived into the `path` layer's row index.
  The docs call row order "engine-defined unless the source query has an `ORDER BY`",
  and the materialisation step gives no ordering guarantee (inferred).
  Layers have their own `ORDER BY` subclause for this.
- The grammar accepts several `VISUALISE` statements after one SQL part,
  and `parse_query` returns one plot per statement.
  `execute_with_reader` keeps only the first (`specs.into_iter().next()`, `src/reader/mod.rs`).

### Representative examples

Verbatim from `README.md`:

```sql
SELECT date, revenue, region
FROM sales
WHERE year = 2024

VISUALISE date AS x, revenue AS y, region AS color
DRAW line
SCALE x
  SETTING breaks => 'month'
LABEL title => 'Sales by Region'
```

Verbatim from `doc/get_started/anatomy.qmd`:

```sql
VISUALISE bill_len AS x, bill_dep AS y, species AS stroke FROM ggsql:penguins
DRAW point 
  MAPPING body_mass AS size
  SETTING fill => null
DRAW smooth 
  SETTING method => 'ols'
SCALE stroke TO dark2
SCALE BINNED size TO (4, 15)
  SETTING breaks => 4
```

Verbatim from `doc/gallery/examples/multi-layer.qmd` (layers from different CTEs):

```sql
WITH temps AS (
  SELECT Date, Temp as value FROM ggsql:airquality
),
ozone AS (
  SELECT Date, Ozone as value FROM ggsql:airquality WHERE Ozone IS NOT NULL
)
VISUALISE
DRAW line
  MAPPING Date AS x, value AS y, 'Temperature' AS color FROM temps
DRAW point
  MAPPING Date AS x, value AS y, 'Ozone' AS color FROM ozone
  SETTING size => 3
SCALE x VIA date
LABEL
  title => 'Temperature vs Ozone',
  x => 'Date',
  y => 'Value'
```

Verbatim from `doc/gallery/examples/faceted.qmd`:

```sql
SELECT bill_len, bill_dep, species, island FROM ggsql:penguins
VISUALISE bill_len AS x, bill_dep AS y
DRAW point
FACET species BY island
LABEL
  title => 'Bill Dimensions by Species and Island',
  x => 'Bill Length (mm)',
  y => 'Bill Depth (mm)'
```

Verbatim from `doc/gallery/examples/pie-chart.qmd`:

```sql
VISUALISE island AS fill FROM ggsql:penguins
  DRAW bar
  PROJECT TO polar
```

Verbatim from `doc/vendor/SKILL.md` (SQL aggregation before `VISUALISE`, two layers):

```sql
-- Lollipop chart
SELECT ROUND(bill_dep) AS bill_dep, COUNT(*) AS n FROM ggsql:penguins GROUP BY 1
VISUALISE bill_dep AS x
DRAW range MAPPING 0 AS ymin, n AS ymax SETTING hinge => null
DRAW point MAPPING n AS y
```

### Parsing, execution and rendering in the core

- The parser is a tree-sitter grammar (`tree-sitter-ggsql/grammar.js`) used from Rust (`src/parser/`).
  It parses the SQL part too, with a permissive token-bag grammar rather than a full SQL grammar.
- `Reader::execute(query)` returns a resolved `Spec`: SQL executed, mappings resolved against the schema,
  statistics and scales applied.
  Dialect-generated SQL (schema probes, statistics, transforms) runs in the database (`src/CLAUDE.md`, "Caching layer").
- `Writer::render(&spec)` produces the output.
  Writers: `vegalite` (default); `svg`, `pdf`, `hep` (default features, no GPU); `png`, `jpeg`, `tiff`, `webp`
  (non-default, need a GPU adapter).
  `hep` is a plot document for a host to lay out and draw itself.
  All but Vega-Lite share one renderer ("hephaestus", an internal name).
  `ggsql view` shows a plot in a native window (feature `window`).
- The ggsql crate bundles its own DuckDB through `duckdb` crate `~1.10502` (DuckDB v1.5.2) for its standalone reader.

### Where the picture is shown

- **CLI** (`ggsql exec`/`run`): Vega-Lite JSON on stdout by default; `-o chart.svg` picks the writer from the extension;
  binary formats are refused on a terminal (`doc/get_started/tooling/cli.qmd`).
- **Jupyter kernel** (`ggsql-jupyter`): renders in the kernel and sends images
  (`image/svg+xml`, `image/png`); plain SQL comes back as an HTML table.
  Positron gets plots in its Plots pane.
- **Python** (`ggsql` 0.3.3): `render_altair(df, viz)` and `reader.execute()` + `VegaLiteWriter().render()`
  return an Altair chart built with `altair.Chart.from_json`; Altair provides the notebook display.
- **R**: `ggsql_execute()` returns a `Spec`; a `ggsql_vega` htmlwidget and a `knit_print.Spec` method display it,
  and a knitr engine runs `ggsql` chunks that can reference R data as `r:mtcars`.
- **WebAssembly** playground: draws with the SVG writer in the browser.

### The DuckDB extension (`posit-dev/ggsql-duckdb`)

- A C++ extension (`src/*.cpp`) with a Rust static library (`rust/`) that depends on ggsql with only `vegalite`,
  so no second DuckDB is linked in.
  It carries a copy of ggsql's DuckDB dialect for that reason.
- **Parsing**: registers a `ParserExtension`.
  In v1.5.x its `parse_function` receives the query string after DuckDB's parser fails,
  scans for a top-level `VISUALISE`/`VISUALIZE` word
  (skipping quotes and comments),
  strips a trailing `;`, and plans a call to the table function `ggsql_run(query)`. ggsql's tree-sitter parser then
  parses the whole statement.
  A scalar `ggsql('<query>')` runs the same pipeline.
- **Data**: the Rust side calls back into C++ (`exec_sql`),
  which runs SQL on a **sibling `Connection`** on the same `DatabaseInstance` and returns an Arrow C stream.
  The reason given: calling back into the issuing `ClientContext` from inside a table function deadlocks.
  Consequence, verified locally: temporary tables and views of the user's session are invisible
  ("Table with name tt does not exist").
- **Output**, chosen by the session setting `ggsql_output`
  (verified locally on DuckDB 1.5.5 with community build v0.4.1):
  - `silent` (default): registers the spec with a `tiny_http` server on `127.0.0.1:<random port>`,
    opens the default browser with `open::that`, and returns no result set (`StatementReturnType::NOTHING`).
  - `url`: same, plus one row with the plot URL.
  - `spec`: the Vega-Lite v6 JSON as `VARCHAR`, data inlined as `data.values`; no server, no browser.
  - `html`: a self-contained page of about 850 KB with vega, vega-lite and vega-embed inlined.
  The result column is always `plot` (`VARCHAR`).
  `GGSQL_NO_OPEN_BROWSER=1` suppresses the browser.
- Its README suggests `COPY (SELECT ggsql('…')) TO 'plot.html'` in `html` mode.
  Locally that writes a CSV: a header line and the HTML in double quotes with doubled inner quotes.
- Known parser gaps: table-function column aliases such as `range(10) t(x)` fail with "Parse tree contains errors"
  (issue #11, verified locally).
  The README's own `SELECT * FROM range(10) t(x) VISUALISE …` examples fail the same way on v0.4.1.
- Issue #12 asks for DuckDB UI support; it is open with no response.
- WebAssembly targets are excluded (`community-extension.yml`).

### DuckDB versions and v2

- ggsql-duckdb main pins DuckDB v1.5.4 in CI and submodules.
  The community repository serves `ggsql` 0.4.1, which installed and ran on DuckDB 1.5.5.
- No maintainer issue or statement about DuckDB v2 was found in `posit-dev/ggsql` or `posit-dev/ggsql-duckdb`.
- Draft PR #17 ("Begin migration to DuckDB 2.0 alpha and upgrade ggsql to 0.5.2"),
  opened 2026-09-27 by an outside contributor (author association `NONE`), has no review yet.
  It pins DuckDB `d8a1bd4f` on `v2.0-cyanoptera` and keeps the legacy `ParserExtension`,
  whose v2 `parse_function` receives a `vector<SimpleToken>`.
  It rebuilds the query text from token spellings
  (no source offsets or whitespace are available) and reports `consumed_tokens` for one statement.
  It does not use PEG grammar extensions.
  Its `docs/DUCKDB_2_MIGRATION.md` notes that ggsql-only spellings
  (`//` comments, backtick identifiers)
  then only work through the scalar `ggsql('…')`, and keeps the sibling-connection limitation.

## the-stats-duck `VISUALIZE`

- Community extension `stats_duck` 0.8.0, Apache-2.0, C++.
  `VISUALIZE` is one feature of a statistics toolkit.
  Its README calls it "deliberately minimal, WebAssembly-friendly" and "not a reimplementation of ggsql".
- Hook: the legacy `ParserExtension` fallback with a hand-written tokenizer (`src/ggsql.cpp`),
  registered through `config.parser_extensions` (≤ v1.4.x) or the callback manager (v1.5.x+).
- **Syntax**: statement-initial, with an optional leading `WITH`.
  The suffix form `SELECT … VISUALIZE …` is a parser error (verified locally).
  Verbatim from `README.md`:

  ```text
  [WITH [RECURSIVE] <cte> AS (...) [, <cte> AS (...)]*]
  VISUALIZE <expr> AS <aesthetic> [: <type>] (, <expr> AS <aesthetic> ...)
  FROM <table>
  DRAW <mark> [STAT <identity|smooth|summary>] (DRAW <mark> [STAT ...])*
  [FACET BY <expr> [ROWS | COLS] | FACET BY <row_expr>, <col_expr>]
  [SCALE <channel> {TO <scheme> | ZERO true|false | DOMAIN <lo> <hi> | LABEL '<text>'}+]*
  [TITLE '<text>' [SUBTITLE '<text>']]
  ```

  Marks: `point`, `line`, `bar`, `histogram`, `text`, `area`, `rule`, `tick`, `errorbar`, `errorband`, `boxplot`,
  `violin`, `heatmap`, `density`, `regression`.
  Other extensions can add marks as scalar functions named `visualize_mark_v1_<name>`.
- **Output**: one row, `spec VARCHAR` (Vega-Lite v5, data referenced by name) and `layer_sqls MAP(VARCHAR, VARCHAR)`.
  The client runs each layer's SQL and feeds vega-embed's `datasets`
  (verified locally, e.g. `layer_sqls = {layer_0='SELECT bill_len AS x, bill_dep AS y FROM penguins'}`).
  Nothing is rendered by DuckDB.
- **Issue #46** ("DuckDB v2.0 readiness:
  PEG grammar hooks for VISUALIZE + stable-ABI port + self-hosted signed repository"),
  opened 2026-08-19 by a collaborator, label `epic`, open.
  It proposes native PEG grammar productions "instead of a whole-statement fallback", a port to the stable C ABI,
  and a signed self-hosted repository.
  All checklist items are unchecked.
  The only comment (2026-09-21) points at the PEG parser blog post as the mechanism to target,
  and records a decision against procedural-SQL features.
  No branch contains PEG work (`v0.9` is five commits ahead of main, none about the parser).

## Other charting UX

"Surface" is how the user asks for a chart; "Delivery" is how the picture reaches them.

| Tool | Surface | Delivery | Link |
|---|---|---|---|
| `ggsql` (DuckDB ext.) | Query suffix `VISUALISE …` or scalar `ggsql('…')` | Side effect: browser via local HTTP server; or URL/spec/HTML as `VARCHAR` | [repo](https://github.com/posit-dev/ggsql-duckdb) |
| `stats_duck` | Statement `VISUALIZE … FROM t DRAW …` | Row of Vega-Lite spec + per-layer SQL; client renders | [repo](https://github.com/KoliStat/the-stats-duck) |
| `anofox_visualization` | Macros `anofox_bar(x, y)` etc. over `list()`, scalar `anofox_render(json)` | SVG as `VARCHAR`, one per group (verified locally) | [repo](https://github.com/DataZooDE/anofox-visualization) |
| `miniplot` | Scalars `bar_chart(labels[], values[], title [, file])` | Opens browser, or writes an HTML file when a path is given | [repo](https://github.com/nkwork9999/miniplot) |
| `textplot` | Scalars `tp_bar`, `tp_sparkline`, `tp_density`, `tp_qr` | Unicode/emoji `VARCHAR` inline in the result grid (verified locally) | [repo](https://github.com/Query-farm/textplot) |
| `duckdbi`, `duckgl`, `dash` | `SELECT duckdbi_start(host, port)`, `PRAGMA dash` | Embedded web app with charts, opened in a browser | [community list](https://duckdb.org/community_extensions/) |
| DuckDB UI | `duckdb -ui` / `.ui_command` | Browser notebook; ggsql-duckdb#12 asks for plots there, unanswered | [docs](https://duckdb.org/docs/current/core_extensions/ui) |
| ClickHouse `bar`, `sparkbar` | Scalar `bar(x, min, max[, width])`, aggregate `sparkbar(buckets[, min_x, max_x])(x, y)` | Unicode block string in a cell | [sparkbar](https://clickhouse.com/docs/sql-reference/aggregate-functions/reference/sparkbar) |
| Kusto `render` | Last operator: `T \| render timechart with (title=…)` | Annotation on the result; the client draws it | [docs](https://learn.microsoft.com/en-us/kusto/query/render-operator) |
| Malloy | Tag `# bar_chart` on a query or view | Separate Vega-Lite renderer in VS Code and notebooks | [docs](https://docs.malloydata.dev/documentation/visualizations/overview) |
| Mosaic / vgplot | JS API or JSON/YAML spec issuing queries | DuckDB (WASM or server) runs queries; Observable Plot draws SVG | [site](https://idl.uw.edu/mosaic/) |
| psql | `\g \|command`, `\o \|command`, `\pset format csv` | Pipe results to an external program such as gnuplot | [docs](https://www.postgresql.org/docs/current/app-psql.html) |
| usql | Meta-command `\chart CHART [(OPTIONS)]` | Inline image via Kitty/iTerm/Sixel; ECharts in an embedded JS engine, build tag `charts` | [repo](https://github.com/xo/usql) |
| harlequin | TUI SQL IDE | No chart feature mentioned in its README | [repo](https://github.com/tconbeer/harlequin) |
| VisiData | Keys: `!` marks the x column, `.` graphs a column | Plot drawn inside the TUI | [docs](https://www.visidata.org/docs/graph/) |
| YouPlot (`uplot`) | `duckdb -csv … \| uplot bar -d,` | Unicode plot on the terminal from stdin | [repo](https://github.com/red-data-tools/YouPlot) |
| termgraph | `termgraph data.dat` | Bar charts on the terminal from a file | [repo](https://github.com/mkaz/termgraph) |
| gnuplot | `duckdb -csv … \| gnuplot -e "plot '-' …"` | Terminals `dumb`/`block` (text), `sixelgd`, `kittycairo` (inline images); pipe verified locally | [site](http://www.gnuplot.info/) |
| lets-plot | ggplot-style Python/Kotlin API | Notebook display; files via `ggsave` | [repo](https://github.com/JetBrains/lets-plot) |
| JupySQL `%sqlplot` | `%sqlplot histogram --table t --column x` | Aggregates in SQL, returns a matplotlib `Axes` | [docs](https://jupysql.readthedocs.io/en/latest/plot.html) |
| dbplot (R) | `dbplot_histogram(tbl, x)` etc. on a database table | Aggregates in the database, plots with ggplot2 | [CRAN](https://cran.r-project.org/package=dbplot) |

No R package named `sqlplot` was found; JupySQL's `%sqlplot` and dbplot are the nearest matches.

## DuckDB CLI: ways a chart can reach the user

Checked against `duckdb -help`, `.help -all` and local runs of DuckDB 1.5.5.

### Output redirection and external programs

- `.output FILE` / `.once FILE` send output to a file; a FILE starting with `|` opens a pipe.
  `.once '|cat > piped.svg'` in `list` mode with headers off wrote a multi-line SVG string unchanged (verified locally).
- `.once -x` / `.excel` and `.once -e` exist.
  Both write a temp file (`/tmp/temp<hex>.csv` or `.txt`) and run `xdg-open` on it
  (`open` on macOS, `start` on Windows).
  `-e` does **not** use `$EDITOR` for this: with a stub `xdg-open` on `PATH`, both calls went to `xdg-open`
  (verified locally, matches `ShellState::ResetOutput` in `tools/shell/shell.cpp`).
  `-e` writes the current mode's rendering
  (a duckbox table by default) with a `.txt` suffix, so the suffix cannot be chosen.
- `.shell CMD` / `.system CMD` run a shell command; `.pager` pipes long results into a pager.
  `.edit` opens the query (not the result) in `DUCKDB_EDITOR`/`EDITOR`/`VISUAL`.
- `-ui` / `.ui_command` launch the browser UI.
- `-safe` disables `.output`, `.once`, `.excel`, `.shell` and `.system` (verified locally).
- Modes: `ascii`, `box`, `csv`, `column`, `duckbox` (default), `html`, `insert`, `json`, `jsonlines`, `latex`,
  `line`, `list`, `markdown`, `quote`, `table`, `tabs`, `tcl`, `trash`.
- No way for an extension to add dot commands was found.
  The CLI's built-in shell extension registers `getenv(name)`
  and sets `duckdb_api` to `cli` (`tools/shell/shell_extension.cpp`).
  `current_setting('duckdb_api')` returned `cli` in the CLI and `python/3.14` from Python (verified locally).

### How values are rendered (verified locally)

- **BLOB**: shown as escaped text in every mode, printable ASCII as-is and other bytes as `\xNN`,
  e.g. `\x89PNG\x0D\x0A\x1A\x0A`; `json` doubles the backslashes, `quote` wraps it in single quotes.
  `.binary on` changes nothing.
  Raw image bytes therefore never reach the terminal or a pipe from a `SELECT`.
- **Control characters**: `duckbox` escapes every byte below 32
  (`\n`, `\t`, `\e`, …)
  in `ConvertRenderValue` (`src/common/box_renderer.cpp`),
  so a value cannot span lines or carry terminal escape sequences there.
  `list`, `line`, `csv` and `markdown` pass newlines and `ESC` through unchanged; `json` escapes them.
- **Long VARCHAR** in `duckbox`:
  - With a terminal, the width defaults to the terminal width.
    A single long value is wrapped over several lines and cut off with `…` once it reaches the `.maxrows` limit
    (default 40, leaving 36 value lines in a one-row result).
    In a 100-column pseudo-terminal a 5000-character string showed about 3,500 characters.
  - `.maxwidth N` sets the width explicitly; `.maxrows` raises or lowers the line budget.
  - When stdout is not a terminal, there is no width limit: the 5000-character value printed on one line.
  - `.last` re-renders the previous result without truncation.
- `COPY (SELECT …) TO 'f' (FORMAT blob)`:
  - Writes the raw bytes: an 8-byte PNG signature came out as exactly those 8 bytes.
  - Needs exactly one column of type `BLOB`; a `VARCHAR`, or a second column, fails with
    `"COPY (FORMAT BLOB)" only supports a single BLOB column`.
  - Several rows are concatenated into one file with no separator.
  - `PARTITION_BY (name)` writes one file per partition (`parts/name=f0/data_0.blob`).
  - The docs add compression detection from the file extension (`.blob.gz`, `.blob.zst`).

### Python client

- `duckdb` 1.5.5: `DuckDBPyRelation` and `DuckDBPyConnection` define `__repr__` (the duckbox text) and no
  `_repr_html_`, `_repr_png_` or `_repr_mimebundle_` (verified locally).
- `BLOB` values fetch as Python `bytes`.
  A custom type over `BLOB` therefore reaches Python as plain `bytes` with no display hook (inferred for custom types).
  Notebook display needs a Python-side wrapper.

## Custom-type display

- **CLI row modes** (`list`, `csv`, `line`, …) convert each column with
  `VectorOperations::Cast(*state.conn->context, …)` to `VARCHAR` (`ShellRenderer::ConvertChunk`,
  `tools/shell/shell_renderer.cpp`, v1.5.5).
- **duckbox** does the same (`VectorOperations::Cast(context, …)`, `src/common/box_renderer.cpp`), then escapes
  control characters; JSON-typed and nested columns get pretty-printing and highlighting.
- `VectorOperations::Cast(ClientContext &, …)` looks up `DBConfig::GetCastFunctions()`,
  the database-wide cast set that extension-registered casts join (`src/common/vector_operations/vector_cast.cpp`).
- **json mode** first casts nested types (`STRUCT`, `LIST`, `MAP`) to `JSON`, then to `VARCHAR`.
  A `STRUCT`-backed custom type therefore appears as its raw struct there.
- Observed on 1.5.5 (verified locally):
  - `INET` (inet extension, alias over `STRUCT`) shows as `192.168.0.0/24` in duckbox, `list` and `csv`,
    but as `{'ip_type': 1, 'address': 3232235520, 'mask': 24}` in `json`.
  - `POINT_2D` (spatial, alias over `STRUCT`) shows as `POINT (1 2)` in `csv`.
  - `GEOMETRY` shows as WKT (`POINT (1 2)`).
  - `JSON` (alias over `VARCHAR`) shows as its text.
  - `WKB_BLOB` (spatial, alias over `BLOB`, no custom `VARCHAR` cast) shows the `\xNN` escape,
    identical to its explicit `::VARCHAR`; the header shows the alias name `wkb_blob`.
  - `CREATE TYPE chart AS BLOB` from SQL resolves to plain `BLOB` (`typeof` returns `BLOB`).
- So an extension controls display of a custom type by registering a cast from it to `VARCHAR`.
  There is no separate display hook: `duckdb_types()` lists types
  but carries no rendering information. plume' previous attempt (commit `3cc964dc`) did this through the C API
  (`duckdb_register_logical_type`, `duckdb_register_cast_function`);
  its commit message reports `CHART(line, 2 series, 240 points)` as the display.
- On `v2.0-cyanoptera` (`d8a1bd4f`), duckbox calls `BoxRendererContext::CastToVarchar`,
  implemented by `VectorOperations::TryCast(context, …)` (`src/common/client_box_renderer_context.cpp`),
  and the shell's row modes still call `VectorOperations::Cast(*state.conn->context, …)`.
  The v2 C API has stable custom types (`custom_type_*`) and cast functions
  (`cast_function_*`, stable in v2.0.0),
  and its spec describes custom types as "logically distinct so it can carry its own cast functions".

## Inferred, not verified

- ORDER BY before `VISUALISE` is not guaranteed to carry through ggsql's temp-table materialisation.
  Only one small case was tested.
- A chart value encoded as a terminal-graphics escape sequence
  (Sixel, Kitty, iTerm2)
  in a `VARCHAR` would display in `list` or `line` mode in a capable terminal, but not in `duckbox`.
  Not tried with a real image.
- `duckdb_api` and `getenv('TERM')` would let an extension adapt its output to the CLI and terminal.
  No extension doing this was found.
- ggsql-duckdb's sibling-connection limitation comes from re-running SQL from inside a table function.
  An aggregate that consumes the query's rows directly, as in the plume plan, does not need a second connection.

## Sources

- [ggsql](https://github.com/posit-dev/ggsql) and [ggsql.org](https://ggsql.org/syntax/); files named above,
  notably `tree-sitter-ggsql/grammar.js`, `doc/syntax/`, `doc/get_started/`, `src/CLAUDE.md`,
  `src/reader/mod.rs`, `src/execute/mod.rs`, `src/execute/cte.rs`, `CHANGELOG.md`
- [ggsql alpha announcement](https://opensource.posit.co/blog/2026-04-20_ggsql_alpha_release/)
- [ggsql-duckdb](https://github.com/posit-dev/ggsql-duckdb): `README.md`, `CLAUDE.md`, `src/ggsql_parser.cpp`,
  `src/ggsql_exec.cpp`, `src/ggsql_extension.cpp`, `rust/src/lib.rs`, `test/sql/ggsql.test`;
  [issue #11](https://github.com/posit-dev/ggsql-duckdb/issues/11),
  [issue #12](https://github.com/posit-dev/ggsql-duckdb/issues/12),
  [PR #17](https://github.com/posit-dev/ggsql-duckdb/pull/17)
- [ggsql-python](https://github.com/posit-dev/ggsql-python), [ggsql-r](https://github.com/posit-dev/ggsql-r)
- [the-stats-duck](https://github.com/KoliStat/the-stats-duck): `README.md`, `docs/visualize.md`, `src/ggsql.cpp`;
  [issue #46](https://github.com/KoliStat/the-stats-duck/issues/46)
- [duckdb/community-extensions](https://github.com/duckdb/community-extensions): `extensions/*/description.yml`
- DuckDB docs: [CLI dot commands](https://duckdb.org/docs/current/clients/cli/dot_commands),
  [CLI output formats](https://duckdb.org/docs/current/clients/cli/output_formats),
  [COPY](https://duckdb.org/docs/current/sql/statements/copy)
- DuckDB source (v1.5.5): `src/common/box_renderer.cpp`, `src/common/vector_operations/vector_cast.cpp`,
  `tools/shell/shell.cpp`, `tools/shell/shell_renderer.cpp`, `tools/shell/shell_extension.cpp`;
  (`v2.0-cyanoptera`): `src/common/box_renderer.cpp`, `src/common/client_box_renderer_context.cpp`,
  `tools/shell/shell_renderer.cpp`, `api_spec/v2/logical_type/custom_type.yaml`, `api_spec/v2/function/cast.yaml`
- Tools in the table: links in the table rows
