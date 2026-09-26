# duckers

duckers is a DuckDB extension for drawing charts from SQL with the Rust plotting library
[`plotters`](https://github.com/plotters-rs/plotters).
It is written in Rust against DuckDB's C extension API
and started from [`duckdb/extension-template-rs`](https://github.com/duckdb/extension-template-rs),
so building it needs no DuckDB source tree and no C or C++ code.

## Status

Early.
Line, point (scatter) and bar charts render to SVG and PNG.
Styling beyond captions, axis descriptions and ranges is not there yet.

## Usage

Charts are values of type `CHART`.
A series constructor builds one from lists of x and y values,
modifiers named after the plotters builder methods adjust it, and an output function renders it:

```sql
SELECT line_series(list(day ORDER BY day), list(temp ORDER BY day))
         .caption('Oslo temperature')
         .x_desc('day')
         .y_desc('°C')
         .to_svg()
FROM weather
WHERE city = 'Oslo';
```

duckers draws exactly the rows it is given, in list order.
Filtering, aggregating, binning and ordering are done in SQL, usually with `ORDER BY` inside `list()`:

```sql
-- one series per city, with a legend
SELECT line_series(
         list(day ORDER BY city, day),
         list(temp ORDER BY city, day),
         list(city ORDER BY city, day)
       ).to_png(1024, 768)
FROM weather;

-- bars from rows aggregated in SQL
SELECT bar_series(list(city ORDER BY city), list(avg_temp ORDER BY city)).to_png()
FROM (SELECT city, avg(temp) AS avg_temp FROM weather GROUP BY city);

-- several series kinds on one chart
SELECT line_series(list(i), list(i * i))
         .draw_series(point_series(list(i), list(i * i)))
         .to_svg()
FROM range(10) t(i);
```

Selecting a `CHART` shows a summary such as `CHART(line, 2 series, 240 points)`.

### Functions

| Function | plotters counterpart | Description |
|---|---|---|
| `line_series(xs, ys [, label \| labels])` | `LineSeries` | Line chart. `xs` is `DOUBLE[]` or `TIMESTAMP[]`, `ys` is `DOUBLE[]`. |
| `point_series(xs, ys [, label \| labels])` | `PointSeries` | Scatter plot, same arguments as `line_series`. |
| `bar_series(categories, ys)` | `Histogram` | Bar chart over a `VARCHAR[]` of categories. |
| `caption(chart, text [, size])` | `ChartBuilder::caption` | Chart title; size defaults to 30. |
| `x_desc(chart, text)`, `y_desc(chart, text)` | `MeshStyle::x_desc`, `y_desc` | Axis descriptions. |
| `x_range(chart, lo, hi)`, `y_range(chart, lo, hi)` | `build_cartesian_2d` ranges | Explicit axis ranges; the default is the data's extent. |
| `draw_series(chart, other)` | `ChartContext::draw_series` | Draws the series of `other` on `chart`. |
| `to_svg(chart [, width, height])` | `SVGBackend` | SVG as `VARCHAR`; 640×480 by default. |
| `to_png(chart [, width, height])` | `BitMapBackend` | PNG as `BLOB`; 640×480 by default. |

`label` is one `VARCHAR` for the whole series.
`labels` is a `VARCHAR[]` with one label per point, which splits the points into one series per label.
Points with a NULL x or y are left out.

### Writing files

`COPY ... (FORMAT blob)` writes a single `BLOB` value to a file as-is:

```sql
COPY (SELECT line_series(list(i), list(i)).to_png() FROM range(10) t(i))
TO 'chart.png' (FORMAT blob);

-- to_svg returns VARCHAR; encode() turns it into a BLOB
COPY (SELECT line_series(list(i), list(i)).to_svg().encode() FROM range(10) t(i))
TO 'chart.svg' (FORMAT blob);
```

### Why lists and not aggregates

The natural shape would be an aggregate, `line_series(x, y ORDER BY x)`.
DuckDB's C API crashes on `ORDER BY` inside C API aggregates
([duckdb/duckdb#26109](https://github.com/duckdb/duckdb/issues/26109)), so the constructors take lists from `list()`,
which supports `ORDER BY` safely.

### Fonts

PNG rendering uses the embedded DejaVu Sans font
(`assets/fonts`, see `DejaVuSans-LICENSE`), so no system fonts are needed.
SVG output leaves text rendering to the viewer.

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
