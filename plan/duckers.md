# Plan: duckers, plotters charts for DuckDB v2

duckers lets SQL users draw charts with Rust's [`plotters`](https://github.com/plotters-rs/plotters) from inside DuckDB
v2, see them inline in the terminal or in a window, and write them as SVG or PNG.
Its API mirrors plotters, so a plotters user who knows the rules in [The duckers paradigm](#the-duckers-paradigm) can
write the SQL for a chart without a reference.
A `VISUALIZE` clause is optional sugar on top and comes last.

This document decides the user API and the behaviour, and lays out the roadmap.
The research behind it is in [`docs/reports/`](../docs/reports/):

- [C API from Rust](../docs/reports/2026-09-27-duckdb-v2-c-api-from-rust.md) and
  [parser extensibility](../docs/reports/2026-09-27-duckdb-v2-parser-extensibility.md): the two-extension split.
- [v2 preview builds and C API details](../docs/reports/2026-09-27-duckdb-v2-preview-and-capi-details.md):
  what the v2 C API can and cannot do, verified with a probe extension.
- [plotters API inventory](../docs/reports/2026-09-27-plotters-api-inventory.md): every name the SQL API mirrors.
- [Native window display](../docs/reports/2026-09-27-native-window-display.md): how a chart reaches the user.
- [ggsql and prior art](../docs/reports/2026-09-27-ggsql-and-prior-art.md): what others chose.

## Goals and non-goals

Goals:

- Every 2D chart plotters can draw is reachable from SQL, in the same vocabulary.
- Output as SVG (`VARCHAR`) and PNG (`BLOB`), files through `COPY`, and a `show()` that opens a viewer.
- The extension is built once per platform against the stable v2 C API and keeps loading across DuckDB releases.
- Each milestone ships something usable on its own.

Non-goals:

- A grammar-of-graphics layer (ggsql, ggplot).
  SQL does the statistics and reshaping; duckers only draws rows.
- Interactivity (zoom, hover). plotters is a static renderer.
- 3D charts. plotters supports them and they can be added later, but they are not on the roadmap.
- Theming systems beyond what plotters exposes.

## The duckers paradigm

These rules let a plotters user predict the SQL.
They are the contract; the tables below follow from them.

1. **Builders are values.**
   A plotters builder (`ChartBuilder`/`ChartContext`, `MeshStyle`, `SeriesLabelStyle`, a series, a `FontDesc`) is a SQL
   value of a custom type (`CHART`, `MESH`, `SERIES_LABELS`, `SERIES`, `FONT`).
   Nothing is drawn until an output function such as `to_svg` or `show` runs.
2. **Names are plotters names.**
   Every function is the snake_case name of the plotters method or type it maps to,
   with the value as the first argument, so DuckDB's dot-call syntax
   (`x.f(y)` is `f(x, y)`)
   makes the SQL read like the Rust chain: `ChartBuilder::caption` is `caption`, `LineSeries` is `line_series`,
   `MeshStyle::x_desc` is `x_desc`.
   Where the name is taken by a DuckDB built-in that an extension cannot overload, the plotters receiver qualifies it:
   `Histogram::vertical` is `histogram_vertical`, `root.fill` is `root_fill`.
3. **Rows become series through aggregates.**
   `LineSeries::new(iter, style)` consumes an iterator; `line_series(x, y)` consumes the rows of a group.
   A SQL group has no order, so the aggregate sorts by `x` unless `order_by := expr` says otherwise.
   `GROUP BY` makes one series per group; `key := expr` splits one aggregate into a list of series,
   one per distinct key.
4. **The chain is the Rust chain.**
   `configure_mesh()` returns a `MESH`, `configure_series_labels()` returns a `SERIES_LABELS`,
   and `draw()` on either returns the `CHART`.
   Draw order is chain order: a series drawn after the mesh paints over it, exactly as in plotters.
5. **Closures become data.**
   Where plotters takes a closure, SQL takes a value: a format string for a label formatter,
   a marker name for a point element, a colour for a style function.
   Legend glyphs are derived from the series kind instead of being drawn by a closure.
6. **Styles are strings and small values.**
   Anywhere plotters takes `Into<ShapeStyle>` or a `Color`, SQL takes a colour string,
   optionally followed by a stroke width: `light_line_style('grey')`, `axis_style('black', 2)`.
   Anywhere plotters takes `IntoTextStyle`, SQL takes a size, or a `FONT` built with `font(family, size)`.
   `mix('white', 0.8)` mirrors `WHITE.mix(0.8)`.
7. **The coordinate system follows the data types.**
   Numeric x is a continuous axis, `DATE`/`TIMESTAMP` x a time axis, `VARCHAR` x a category axis.
   `x_range`/`y_range` and `x_log_scale`/`y_log_scale` stand in
   for the range expressions passed to `build_cartesian_2d`.
8. **Defaults exist only where plotters would otherwise draw nothing useful.** plotters requires explicit ranges,
   sized label areas, a mesh call, a series style and a legend call. duckers infers ranges from the data,
   sizes label areas, draws a default mesh first, colours series from `Palette99` in order,
   and draws a legend when a series has a label.
   Every one of these can be overridden with the plotters call.
   All other defaults (font sizes, grid colours, tick sizes, legend position) are plotters' own.
9. **Output is the backend.**
   `to_svg(w, h)` is `SVGBackend`, `to_png(w, h)` is `BitMapBackend`, `show()` is a viewer, and `COPY` writes files.

## The user API

### Types

| Type | plotters counterpart | Made by | Consumed by |
|---|---|---|---|
| `SERIES` | `LineSeries`, `PointSeries`, `Histogram`, `AreaSeries`, ... plus their `ShapeStyle` and `SeriesAnno` | series aggregates | `draw_series` |
| `CHART` | `ChartBuilder` + `ChartContext` + the root `DrawingArea` | `chart()` | outputs, `configure_*`, `draw_series` |
| `MESH` | `MeshStyle` | `configure_mesh(chart)` | `draw` |
| `SERIES_LABELS` | `SeriesLabelStyle` | `configure_series_labels(chart)` | `draw` |
| `FONT` | `FontDesc`/`TextStyle` | `font(family, size [, style])` | any text-style parameter |

All are custom logical types over `BLOB` (a magic prefix, a format version, then a postcard-encoded spec).
The v2 C API registers such types with `custom_type_*`; `typeof()` reports the name and overloads dispatch on it.
Casting any of them to `VARCHAR` gives a one-line summary, e.g. `CHART(line, 2 series, 240 points)`,
and the CLI renders values through that cast in every output mode, so a `SELECT` of a chart is readable in the shell.
Casts to and from `BLOB` are registered explicitly, since the C API provides none by default.
`MESH` and `SERIES_LABELS` carry a copy of the chart they were taken from,
which is what lets `draw()` hand the chart back.

### Series aggregates

| SQL | plotters | Notes |
|---|---|---|
| `line_series(x, y)` | `LineSeries::new(iter, style)` | One polyline through the points. |
| `point_series(x, y)` | `PointSeries::new(iter, size, style)` | One marker per point. |
| `histogram_vertical(bucket, value)` | `Histogram::vertical(&chart).data(iter)` | Sums `value` per distinct `bucket`; `histogram_vertical(x, 1)` counts rows. |
| `histogram_horizontal(bucket, value)` | `Histogram::horizontal(&chart).data(iter)` | The same with horizontal bars. |
| `area_series(x, y)` | `AreaSeries::new(iter, baseline, style)` | Polygon from the points down to the baseline (0). |
| `dashed_line_series(x, y)` | `DashedLineSeries::new(points, size, spacing, style)` | |
| `error_bar_vertical(x, min, avg, max)` / `error_bar_horizontal(y, min, avg, max)` | `ErrorBar::new_vertical` / `new_horizontal` per row | One element per row. |
| `candle_stick(x, open, high, low, close)` | `CandleStick::new(...)` per row | |
| `boxplot_vertical(key, value)` / `boxplot_horizontal(key, value)` | `Boxplot::new_vertical(key, &Quartiles::new(values))` | One box per distinct key; quartiles computed by duckers. |

Every series aggregate also accepts two named parameters:

- `key := expr`: split the rows by `expr`.
  Each distinct value becomes one series labelled `key::VARCHAR`, and the aggregate returns `SERIES[]` ordered by key.
  Per-series styling of the list uses DuckDB's list lambdas: `list_transform(ss, lambda s: s.stroke_width(2))`.
- `order_by := expr`: the drawing order within a series
  (the point order for lines and areas, the bucket order for histograms and boxplots).
  The default is `x` (or `bucket`, or `key`), ascending,
  with ties broken by `y` so the result is deterministic. plotters draws in iterator order; SQL has no iterator order,
  so this is the replacement.

`ORDER BY` inside the call (`line_series(x, y ORDER BY t)`) is **not** the way to order.
DuckDB v2 crashes on `ORDER BY` inside C-API aggregates
([duckdb#26109](https://github.com/duckdb/duckdb/issues/26109), open),
so duckers declares its aggregates order-independent, and the planner then drops such an `ORDER BY` silently.
The same bug crashes `series_agg(...) OVER ()`; window frames are unsupported until the fix lands.

Argument types:

- `x`: any numeric type (continuous axis, mapped to `DOUBLE`), `DATE`, `TIMESTAMP`, `TIMESTAMPTZ` and the
  `TIMESTAMP_*` variants (time axis), or `VARCHAR` (category axis).
- `y`, `value`, `min`, `avg`, `max`, `open`, `high`, `low`, `close`: any numeric type.
- `bucket`: `VARCHAR` (one band per category), any integer type (a segmented integer axis from min to max), or `DATE`
  (one band per day).
  `DOUBLE` buckets are rejected with a hint to bin first or to use `.step()` once it exists.
- `key`, `order_by`: any type.

The v2 C API allows one aggregate per name,
so every parameter is declared `ANY`
and the concrete types are checked in the bind callback with the same error messages a typed overload would give.

Rows where `x` or `y` (or any value argument) is `NULL`,
`NaN` or infinite are skipped. plotters maps `NaN` to the axis origin, which draws a spike,
so filtering is the safe choice.
A gap-at-NULL option (`break_at_null()`) can be added later by splitting the polyline.

### Series methods

These are scalar functions on `SERIES` and return `SERIES`.

| SQL | plotters | Applies to | Default |
|---|---|---|---|
| `style(color [, stroke_width])` | the `Into<ShapeStyle>` argument, `Histogram::style`, `SurfaceSeries::style` | all | `Palette99::pick(i)` for series `i` of the chart, stroke width 1 |
| `stroke_width(px)` | `ShapeStyle::stroke_width` | all | 1 |
| `filled()` | `ShapeStyle::filled` | point markers, line markers, histogram, candle bodies, error-bar dots | histogram filled, others not |
| `label(text)` | `SeriesAnno::label` | all | none (series not in legend) |
| `point_size(px)` | `LineSeries::point_size` | `line_series` | 0 |
| `size(px)` | the `size` argument of `PointSeries::new` | `point_series` | 3 |
| `marker(name)` | the element type parameter of `PointSeries::new` (`'circle'`, `'cross'`, `'triangle'`, `'pixel'`) | `point_series` | `'circle'` |
| `margin(px)` | `Histogram::margin` | histograms | 5 |
| `baseline(v)` | `Histogram::baseline`, the `baseline` argument of `AreaSeries::new` | histograms, `area_series` | 0 |
| `border_style(color [, stroke_width])` | `AreaSeries::border_style` | `area_series` | transparent |
| `size(px)`, `spacing(px)` | the `size` and `spacing` arguments of `DashedLineSeries::new` | `dashed_line_series` | 5, 5 |
| `width(px)` | the `width` argument of `ErrorBar`, `CandleStick`, `Boxplot::width` | error bars, candles, boxplots | 10 |
| `gain_style(color)`, `loss_style(color)` | `CandleStick::new` arguments | `candle_stick` | green, red |

The legend glyph for a labelled series is derived from its kind: a short line
(with a marker if `point_size` is set),
a marker, or a filled rectangle. plotters requires a closure here (`SeriesAnno::legend`); there is no SQL equivalent,
and none is planned.

### Chart builder

`chart()` returns an empty `CHART` and stands for `ChartBuilder::on(&root)`.
These scalar functions take and return `CHART`.

| SQL | plotters | Default |
|---|---|---|
| `caption(text [, size \| font])` | `ChartBuilder::caption(text, style)` | none |
| `margin(px)`, `margin_top(px)`, `margin_bottom(px)`, `margin_left(px)`, `margin_right(px)` | same | 10 all round (plotters: 0) |
| `x_label_area_size(px)`, `y_label_area_size(px)`, `top_x_label_area_size(px)`, `right_y_label_area_size(px)` | same | bottom 30, left 40, top 0, right 0 (plotters: all 0) |
| `set_all_label_area_size(px)`, `set_left_and_bottom_label_area_size(px)` | same | |
| `x_range(lo, hi)`, `y_range(lo, hi)` | the range arguments of `build_cartesian_2d` | the data extent, see [Coordinates](#coordinates-and-ranges) |
| `x_log_scale([base])`, `y_log_scale([base])` | `(lo..hi).log_scale().base(b)` | linear |
| `root_fill(color)` | `root.fill(&WHITE)` | `'white'` (plotters: black bitmap, transparent SVG) |
| `draw_series(series \| series[])` | `ChartContext::draw_series` | |
| `configure_mesh()` | `ChartContext::configure_mesh()` | returns `MESH` |
| `configure_series_labels()` | `ChartContext::configure_series_labels()` | returns `SERIES_LABELS` |
| `set_secondary_coord()` | `ChartContext::set_secondary_coord(x, y)` with ranges from the secondary series | |
| `secondary_x_range(lo, hi)`, `secondary_y_range(lo, hi)` | the range arguments of `set_secondary_coord` | data extent |
| `draw_secondary_series(series \| series[])` | `DualCoordChartContext::draw_secondary_series` | |
| `configure_secondary_axes()` | `configure_secondary_axes()` | returns `MESH` (mesh lines disabled, as in plotters) |

`build_cartesian_2d` itself has no SQL function.
The range expressions it takes are covered by `x_range`/`y_range`,
the log and segmented combinators by `x_log_scale`/`y_log_scale` and the series kinds,
and the value types by the column types.

### Mesh

`configure_mesh(chart)` returns a `MESH`; every method below returns `MESH`; `draw()` returns the `CHART`.
Names, parameters and defaults are plotters' (`MeshStyle`, plotters 0.3.7).

| SQL | Parameters | Default |
|---|---|---|
| `x_desc(text)`, `y_desc(text)` | `VARCHAR` | none |
| `axis_desc_style(size \| font)` | | the label style |
| `x_labels(n)`, `y_labels(n)` | max labels and bold lines | 11 |
| `x_label_formatter(fmt)`, `y_label_formatter(fmt)` | a DuckDB `format()` template for numbers (`'{:.1f} °C'`), a `strftime` pattern for time axes (`'%b %d'`) | the coordinate's formatter |
| `label_style(size \| font)`, `x_label_style(...)`, `y_label_style(...)` | | sans-serif 12, black |
| `x_label_offset(px)`, `y_label_offset(px)` | | 0 |
| `x_max_light_lines(n)`, `y_max_light_lines(n)`, `max_light_lines(n)` | | 10 |
| `light_line_style(color [, w])`, `bold_line_style(color [, w])`, `axis_style(color [, w])` | | `mix('black', 0.1)`, `mix('black', 0.2)`, `'black'` |
| `disable_x_mesh()`, `disable_y_mesh()`, `disable_mesh()` | | mesh drawn |
| `disable_x_axis()`, `disable_y_axis()`, `disable_axes()` | | axes drawn |
| `set_tick_mark_size(position, px)`, `set_all_tick_mark_size(px)` | position `'top'`, `'bottom'`, `'left'`, `'right'` | 5 |
| `draw()` | | returns `CHART` |

If a chain never calls `configure_mesh().draw()`,
duckers draws `configure_mesh().draw()` with defaults before the first series.
A chain that does call it controls the order, as in plotters.

### Legend

`configure_series_labels(chart)` returns a `SERIES_LABELS`; `draw()` returns the `CHART`.

| SQL | Parameters | Default |
|---|---|---|
| `position(name)` | `'upper_left'`, `'middle_left'`, `'lower_left'`, `'upper_middle'`, `'middle_middle'`, `'lower_middle'`, `'upper_right'`, `'middle_right'`, `'lower_right'` | `'middle_right'` |
| `position(x, y)` | `SeriesLabelPosition::Coordinate`, px from the plotting-area origin | |
| `margin(px)` | | 10 |
| `legend_area_size(px)` | | 30 |
| `border_style(color [, w])` | | transparent |
| `background_style(color)` | | transparent |
| `label_font(size \| font)` | | sans-serif 12 |
| `draw()` | | returns `CHART` |

If any series has a `label` and the chain never draws the legend,
duckers draws `configure_series_labels().draw()` with defaults last.

### Styles

- **Colours** are `VARCHAR`: plotters' named colours
  (`'red'`, `'white'`, `'transparent'`, ...),
  the `full_palette` material names (`'blue_400'`, `'deeporange'`), `'#rrggbb'`, `'#rrggbbaa'`, `'rgb(r, g, b)'`,
  `'rgba(r, g, b, a)'`, `'hsl(h, s, l)'`, and `'palette99:n'` for `Palette99::pick(n)`.
  `mix(color, alpha)` returns the colour with its alpha multiplied,
  mirroring `WHITE.mix(0.8)`. v1's parser rejects a dot-call on a bare literal (`'white'.mix(0.8)`); v2's accepts it,
  and `('white').mix(0.8)` works on both.
- **Shape styles** (`Into<ShapeStyle>`) are a colour plus an optional stroke width, as positional arguments.
- **Fonts** (`IntoTextStyle`): `font(family, size [, style])` returns a `FONT`; `style` is `'normal'`, `'bold'`,
  `'italic'` or `'oblique'`; `color(font, color)` sets the colour.
  Any parameter that takes a `FONT` also accepts a bare size (sans-serif), as `IntoTextStyle` accepts a `u32`.
  Bitmaps render text with an embedded font
  (DejaVu Sans, as in the first attempt) through plotters' `ab_glyph` engine, so no system fonts are needed.
  SVG output still needs the embedded font for layout, and writes `family` into the file for the viewer to resolve.
- **Sizes** are integer pixels.
  plotters' relative sizes (`10.percent()`) are not exposed.

### Coordinates and ranges

- The x column type picks the axis: numeric → `RangedCoordf64`; `DATE` → `RangedDate`;
  `TIMESTAMP` and friends → `RangedDateTime`; `VARCHAR` → a category axis (`RangedSlice`, segmented for histograms).
  Every series drawn on one chart must agree on the x kind and on the y kind; a mismatch is an error naming both.
- Default ranges are the data extent over all series on the axis, with no padding (plotters' `fitting_range`).
  Histograms and area series include their baseline.
  An empty chart gets `0..1`.
- `x_range(lo, hi)` with `lo > hi` reverses the axis, as in plotters.
  `lo = hi` is an error.
  The types must match the axis kind (`x_range(DATE '2024-01-01', DATE '2024-02-01')` on a time axis).
- Out-of-range points are clamped to the plotting rectangle, not clipped.
  That is plotters' behaviour and duckers keeps it.
- Category axes list categories in `order_by` order (default: sorted).

### Output and display

| SQL | plotters | Result |
|---|---|---|
| `to_svg(chart [, width, height])` | `SVGBackend::with_string` | `VARCHAR` |
| `to_png(chart [, width, height])` | `BitMapBackend::with_buffer` + PNG encoding | `BLOB` |
| `show(chart, viewer := ..., wait := ..., width := ..., height := ...)` | | shows the chart, see [Display](#display); returns the `CHART` |
| `duckers_set(key, value)`, `duckers_get(key)` | | set or read a process-wide `show()` default, see [Display](#display); `VARCHAR` |
| `COPY (SELECT chart ...) TO 'f.png' (FORMAT png, WIDTH w, HEIGHT h)`, `(FORMAT svg)` | `BitMapBackend::new(path)`, `SVGBackend::new(path)` | one file per chart, one chart per file (M7) |

The default size is 640×480 and the maximum 8192 px per side.
Rendering is deterministic: the same chart value renders to the same bytes, which the tests rely on.

The copy functions take one `CHART` column and write one chart per file: a second row for a file is an error,
as are zero rows and a `NULL` chart.
`PARTITION_BY` is the way to get one file per group; it needs `FILE_EXTENSION 'png'`,
because the C API cannot declare the extension DuckDB puts on partition files:

```sql
COPY (SELECT city, chart().draw_series(line_series(day, temp)) AS chart
      FROM weather GROUP BY city)
TO 'charts' (FORMAT png, PARTITION_BY city, FILE_EXTENSION 'png');
```

Without `FORMAT`, DuckDB infers it from the file extension (`TO 'f.svg'`).
The files are local only (see decision 15).
DuckDB's own `COPY ... (FORMAT blob)` stays the route for remote paths: it takes exactly one `BLOB` column,
so it writes `to_png()` or `to_svg().encode()`.
A `CHART` value passes its check too, because the custom type is a tagged `BLOB`,
and then the file holds the encoded spec, not an image.

### Display

`show()` hands the rendered chart to a viewer.
The viewers, in the order the automatic choice tries them:

| Viewer | How | Where it works | `wait := true` |
|---|---|---|---|
| `'terminal'` | Writes the PNG as a kitty graphics sequence (or iTerm2, or sixel) to the controlling terminal (`/dev/tty`, `CONOUT$` on Windows). The image appears above the result table. | kitty, ghostty, WezTerm, iTerm2, Konsole; Windows Terminal via sixel. Not under tmux, not without a controlling terminal. | n/a, returns at once |
| `'window'` | A native window on its own thread, in-process (`minifb`: no global state, one thread per window). | Linux (X11; Wayland desktops through Xwayland) and Windows, both modes. macOS: only `wait := true`, and only when the call runs on the process main thread (checked at run time); otherwise the next viewer. | blocks until the window is closed |
| `'browser'` | Writes an HTML page to the cache directory and opens the default browser. | Everywhere with a browser. Cannot close the tab when DuckDB exits. | n/a |

Behaviour:

- `viewer` defaults to `'auto'`: the first viewer in the table that can work here.
  The choice uses `getenv('TERM')`-style probing
  (terminal type, `TMUX`, whether `/dev/tty` opens, the current thread)
  and never a query to the terminal, since the shell's line editor owns stdin.
- `wait` defaults to `false`: a window stays open while the shell continues and closes when the process exits.
  With `wait := true` the shell blocks until the window is closed; Ctrl-C does not close it
  (DuckDB offers no interrupt hook to a scalar function), which the docs say plainly.
- The process-wide defaults for `viewer` and `wait` come from the environment
  (`DUCKERS_VIEWER`, `DUCKERS_WAIT`)
  and can be changed in a session with `duckers_set('viewer', 'browser')`, which returns the new value;
  `duckers_get('viewer')` reads one.
  They are process-wide because a v2 C-API extension cannot register `SET` options;
  the C++ shim could add real options later.
  An invalid environment value is an error from every `show()` that needs it, until `duckers_set` replaces it.
- A named argument left out or passed as `NULL` takes its default (the setting, or 640×480);
  only a `NULL` chart gives `NULL`.
- `show()` is volatile, so the optimizer neither folds it at plan time nor caches it.
  A multi-row result shows every row, capped by `duckers_set('max_show', n)` (default 10; `0` shows nothing).
  The cap counts per `show()` call in a planned statement, in the call's bind data:
  every statement the shell or a client plans starts at zero,
  and a prepared statement keeps one count across its executions.
  Rows past the cap are returned unchanged.
- A subprocess viewer (a helper binary owning its own main thread) is the only route to a detached native window on
  macOS.
  It is deliberately not on the roadmap:
  shipping a second binary inside a `.duckdb_extension` runs into Apple Silicon signing
  and managed-Windows application control, and the browser covers the case.
  A user-installed `duckers-view` on `PATH` could be added as a fourth viewer without changing the API.

### Worked examples

A plotters line chart with a caption, axis descriptions, a labelled series and a legend:

```rust
let root = BitMapBackend::new("oslo.png", (800, 600)).into_drawing_area();
root.fill(&WHITE)?;
let mut chart = ChartBuilder::on(&root)
    .caption("Oslo temperature", ("sans-serif", 30))
    .margin(10)
    .x_label_area_size(30)
    .y_label_area_size(40)
    .build_cartesian_2d(first..last, -10.0..30.0)?;
chart.configure_mesh().x_desc("day").y_desc("°C").draw()?;
chart
    .draw_series(LineSeries::new(rows.iter().map(|r| (r.day, r.temp)), &RED))?
    .label("temp")
    .legend(|(x, y)| PathElement::new([(x, y), (x + 20, y)], &RED));
chart.configure_series_labels().border_style(&BLACK).draw()?;
root.present()?;
```

The same chart in duckers:

```sql
COPY (
    SELECT chart()
             .caption('Oslo temperature', 30)
             .margin(10)
             .x_label_area_size(30)
             .y_label_area_size(40)
             .y_range(-10, 30)
             .configure_mesh().x_desc('day').y_desc('°C').draw()
             .draw_series(line_series(day, temp).style('red').label('temp'))
             .configure_series_labels().border_style('black').draw()
    FROM weather
    WHERE city = 'Oslo'
) TO 'oslo.png' (FORMAT png);
```

With the defaults doing their work, one line per city, shown inline in the terminal or in a window:

```sql
SELECT chart().draw_series(line_series(day, temp, key := city)).show() FROM weather;
```

Bars from rows aggregated in SQL, ordered by the count:

```sql
SELECT chart().caption('Articles per section')
         .draw_series(histogram_vertical(section, n, order_by := -n).style('blue_400'))
         .to_svg()
FROM (SELECT section, count(*) AS n FROM articles GROUP BY section);
```

Two series kinds, explicit styles, and a custom legend position:

```sql
SELECT chart()
         .configure_mesh().x_desc('x').y_desc('y').draw()
         .draw_series(line_series(i, i * i).style('red').label('y = x²'))
         .draw_series(point_series(i, i * i).style('black').size(4).filled())
         .configure_series_labels().position('upper_left').background_style(mix('white', 0.8)).draw()
         .to_png(800, 600)
FROM range(10) t(i);
```

A parametric curve, drawn in parameter order:

```sql
SELECT chart().draw_series(line_series(cos(t), sin(t), order_by := t)).show(viewer := 'window', wait := true)
FROM (SELECT i / 20.0 AS t FROM range(126) r(i));
```

## Behaviour

- **One chart per row.**
  A chart expression in a `SELECT` with `GROUP BY` yields one chart per group.
- **Values are immutable.**
  Every method returns a new value; the same `SERIES` can be drawn on several charts.
- **NULL in, NULL out** for every scalar function.
  The exception is `show()`'s named parameters, where `NULL` means the default.
  A series aggregate over zero usable rows returns an empty `SERIES`, which draws nothing.
- **Errors are raised at the call that can detect them.**
  Type mismatches between arguments are bind errors.
  Mixed axis kinds, unknown colour names and bad ranges are runtime errors from the function
  that received the bad value, with the plotters term in the message.
- **Values are versioned.**
  A `CHART` from an older duckers with an incompatible format fails to decode with a clear message.
  Persisting chart values in tables is allowed but not a stability promise before 1.0.
- **Threads.**
  Rendering is pure and runs on whichever thread DuckDB uses.
  Only `show()` and the copy function have side effects.
  The calling thread is unpredictable unless `threads = 1`, which only matters for macOS windows.

## Architecture

Two extensions, as before, but with the weight moved to the core.

1. **Core (Rust, stable C ABI v2).**
   - `duckers-sys`: bindgen over a vendored `duckdb_extension_v2.h` pinned to C API `v2.0.0`.
     The indirection macros do not survive bindgen,
     so the crate stores the function-pointer table returned by `get_api` and calls through it.
     No published Rust crate covers loadable v2 extensions today
     (`libduckdb-sys`'s `capi-v2` binds `duckdb_v2.h` only; `duckdb-neo` is unreleased and links the symbols directly).
   - `duckers`: the `cdylib` with the entrypoint `duckers_init_c_api_v2`, the value types and their casts,
     the aggregates and scalars above, plotters rendering with the embedded font, and the viewers.
     Aggregates take `ANY` parameters with bind-time checks (one aggregate per name), flatten input vectors in `update`
     (v2 does not flatten them), and declare themselves order-independent.
   - The build appends the 512-byte metadata footer (ABI `C_STRUCT`, C API `v2.0.0`, platform) that the loader
     requires even for unsigned loads; extension-ci-tools has no v2 support, so the Makefile carries its own
     footer step.
   - Tests are sqllogictests run with DuckDB's Python runner from the `duckdb --pre` wheel,
     plus Rust unit tests for the spec and the renderer.
     A pinned v2 preview CLI serves manual testing.
2. **Grammar shim (C++, rebuilt per DuckDB version).**
   Adds `VISUALIZE`/`VISUALISE` via PEG rules
   and desugars `<query> VISUALIZE <chart expression>` into `SELECT <chart expression> FROM (<query>)`.
   It contains no rendering and no vocabulary of its own; it is optional and last.

Prior art shaped a few choices: usql's `\chart` shows that inline terminal images work in a SQL shell,
ggsql-duckdb shows a browser viewer and an output-mode setting,
and anofox's SVG-returning functions show that values in the result grid are enough for many uses.

## Roadmap

Each milestone ends with tests against a DuckDB v2 build and a README section for what it adds.
Nothing in a later milestone changes the API of an earlier one; anything that would is a decision to take now.

### M0: spikes

De-risking only, no user-facing output.
Each spike answers a question that a later milestone assumes.

1. A minimal Rust cdylib with the v2 entrypoint loads in the pinned v2 preview
   and registers a scalar with named parameters, an order-independent aggregate with `ANY` and named parameters,
   and a custom type with a `VARCHAR` cast.
   The v2 details report did all of this from C; the spike repeats it from Rust through the bindings.
2. The kitty-graphics writer and a `minifb` window run from a scalar function in the preview CLI on Linux.
   The macOS main-thread check and the Windows `CONOUT$` path are confirmed on real machines,
   or recorded as unconfirmed in M3's acceptance.
3. The build produces a loadable footer without extension-ci-tools.

### M1: skeleton

- `duckers-sys` and the `duckers` crate; entrypoint; `duckers_version()`.
- Build and test tooling: `make` targets for build, footer, test and shell; a uv-managed venv with the v2 wheel
  for the sqllogictest runner; the pinned preview CLI.
- CI matrix for Linux, macOS and Windows builds.
- The custom types exist with `VARCHAR` casts, but no chart functions yet.

### M2: first charts

- `line_series`, `point_series`, `histogram_vertical`, with `key` and `order_by`.
- `chart()`, `draw_series`, `caption`, `x_range`, `y_range`, `root_fill`.
- `configure_mesh` with `x_desc`, `y_desc`, `draw`; the default mesh.
- `label` with the automatic legend; `configure_series_labels` with `position` and `draw`.
- `style`, `stroke_width`, `point_size`, `size`, `filled`; colour strings; default palette.
- `to_svg`, `to_png`; the embedded font.
- Numeric, time and category axes with default ranges.
- Acceptance: the worked examples render; SVG snapshot tests; PNG header tests.

### M3: show

- `show()` with the terminal, window and browser viewers, `viewer`, `wait`, `width`, `height`, `duckers_set`,
  the environment defaults and the multi-row cap.
- Acceptance: in the preview CLI on Linux, the terminal viewer draws above the result table in ghostty and a
  window opens without blocking the shell; `wait := true` blocks; the process exits cleanly with viewers open.
  macOS and Windows behaviour is verified on real machines before the milestone closes, or listed as unverified in
  the README.

### M4: styling breadth

- All `MESH` methods, including formatters, label styles, light/bold line styles and `disable_*`.
- All `SERIES_LABELS` methods.
- `margin*`, `*_label_area_size`, `x_log_scale`, `y_log_scale`.
- `font()`, `FONT` colour and style; `mix`.
- `marker` for points; `histogram_horizontal`; histogram `margin` and `baseline`.

### M5: more series

- `area_series`, `dashed_line_series`, `error_bar_*`, `candle_stick`, `boxplot_*`.
- `histogram_vertical(...).step(s)` (and horizontal) for numeric buckets (`.step(s).use_round().into_segmented()`).
- Time-axis niceties: monthly and yearly key points, `strftime` formatters.

### M6: layout

- Secondary axes: `set_secondary_coord`, `secondary_*_range`, `draw_secondary_series`,
  `configure_secondary_axes`.
- Subplots: `split_evenly(charts, rows, cols)` and `titled(text)` over a list of charts, mirroring
  `DrawingArea::split_evenly` and `titled`.
- `pie(sizes, labels)` as a chart of its own.

### M7: files and hosts

- The copy function: `COPY ... TO 'f.png' (FORMAT png)` and `(FORMAT svg)`, with `PARTITION_BY`.
- Behaviour in Python, R and Node hosts: `show()` picks the browser when there is no terminal, and Jupyter
  display is documented through `to_png` and the client's image display, since the Python client has no display
  hook for custom types.

### M8: VISUALIZE grammar shim

- PEG rules for `VISUALIZE`/`VISUALISE`, desugaring onto the M2 API, enabling via `active_grammar_extensions`,
  and a loader that ensures the core is loaded.
- The shim can also register real `SET duckers_*` options, which the core reads through the context.
- Built per DuckDB version; only started once the grammar-extension API is stable at v2.0 GA.

### M9: distribution

- Core built once per platform; shim per DuckDB version.
- Until the community-extensions repository deploys its v2 leg
  ([community-extensions#2723](https://github.com/duckdb/community-extensions/issues/2723)), releases are GitHub
  assets loaded with `-unsigned`; the submission follows when the leg exists.

## Decisions

Each item records the choice, the alternative, and why.

1. **Function API first, `VISUALIZE` last.**
   The first plan led with the clause.
   The clause needs the version-locked C++ shim and an unstable API,
   and adds no expressiveness over the function API it desugars to.
   Alternative: keep the clause as the primary surface with a ggsql-like grammar.
   Rejected because a second vocabulary contradicts the plotters-naturalness goal.
2. **Aggregates with `key :=` and `order_by :=`, not `ORDER BY` and not lists.**
   `ORDER BY` inside a C-API aggregate segfaults on v2 and v1.5
   (duckdb#26109, open, no fix in sight), and an aggregate cannot see its input order.
   Named parameters with defaults are verified to work on v2 C-API aggregates.
   Alternative: the first attempt's `line_series(list(x ORDER BY x), list(y ORDER BY x))`.
   Rejected because it doubles every argument and hides the plotters shape.
   If the bug is fixed, `ORDER BY` inside the call can be honoured as a second way to order.
3. **Sub-builders carry the chart.**
   `configure_mesh()` returns a `MESH` that includes the chart, and `draw()` returns the chart.
   Alternative: flatten mesh and legend methods onto `CHART` with prefixes where names collide (`legend_margin`).
   Rejected because the literal chain is what a plotters user types, and typed sub-builders avoid every name collision
   (`margin` exists on `ChartBuilder`, `SeriesLabelStyle` and `Histogram`).
4. **`style(color)` rather than `color(color)`.** plotters passes an `Into<ShapeStyle>` and calls the setter `style`
   where one exists (`Histogram`, `SurfaceSeries`).
   `color` would be the more guessable SQL name; it loses to the naming rule.
5. **Defaults that plotters lacks.**
   Ranges, label areas, the mesh, series colours, series order and the legend get defaults (paradigm rule 8).
   Alternative: require them, as plotters does.
   Rejected because `chart().draw_series(line_series(x, y)).show()` must produce a readable chart.
6. **Terminal first, window second, browser third; no helper binary.**
   Inline kitty graphics are verified inside the real DuckDB shell and need no window system or extra thread.
   In-process windows are verified on Linux from any thread and inferred for Windows;
   on macOS a detached in-process window is impossible because the shell's main thread never runs a run loop.
   Alternative: a subprocess viewer for uniform native windows.
   Rejected for now because of packaging
   (signing on Apple Silicon, application control on managed Windows); the browser covers macOS detached mode.
7. **Custom types over `BLOB`, postcard-encoded.**
   Alternative: JSON, which is inspectable with `::JSON`.
   Postcard is smaller and already implemented; a `to_json(chart)` for inspection can be added if wanted.
8. **`-sys` by bindgen over the vendored header.**
   Alternative: generate bindings and a safe wrapper from the YAML spec, which carries ownership and lifecycle metadata.
   Deferred: bindgen is a day's work and the wrapper stays small; the YAML generator pays off only if the wrapper grows,
   and can replace the bindgen output later without touching callers.
   Depending on `libduckdb-sys`'s `capi-v2` is out: it has no extension table or entrypoint.
9. **Series returned by a keyed aggregate are a `SERIES[]`**, not a compound `SERIES`.
   Lists compose with DuckDB's list functions and lambdas for per-series styling;
   a compound value would need its own indexing API.
10. **No `build_cartesian_2d` function.**
    Ranges are inferred and overridden per axis;
    a four-argument function would duplicate `x_range`/`y_range` without carrying the type information the Rust ranges
    carry.
11. **`show()` options are named parameters; defaults are process-wide.**
    The C API cannot register `SET` options, and `getvariable()` is not readable from a function.
    Named parameters are DuckDB-idiomatic
    (table functions use them),
    and `duckers_set` plus environment variables cover session defaults until the shim can register real options.
12. **Names taken by DuckDB built-ins are qualified with the plotters receiver.**
    DuckDB has an aggregate `histogram` and a window function `fill`.
    The v2 C API cannot add an overload to an aggregate
    (registration fails with "GetAlterInfo not implemented"),
    and a scalar cannot share a name with a window function,
    so `histogram(bucket, value)` and `fill(color)` cannot be registered.
    They are `histogram_vertical` (`Histogram::vertical`,
    and `histogram_horizontal` for `Histogram::horizontal` in place of a `.horizontal()` method)
    and `root_fill` (`root.fill`).
    Scalars that share a name with a DuckDB scalar, such as `position`, are overloads and keep their plotters names.
    Alternative: `histogram_series` or `bar_series`; rejected because they name no plotters item.
13. **Linux windows use X11 only; Wayland desktops reach it through Xwayland.**
    The Wayland backend of minifb, as minifb builds it, loads the system libwayland.
    Each closed window then prints about 20 lines of `queue … destroyed while proxies still attached` into the shell.
    Built without libwayland it closes silently but links `libxkbcommon.so.0`,
    and the extension would then fail to load where that library is missing.
    Alternatives: a process-wide libwayland log handler or a stderr redirect
    (both reach into the host process), or a patched minifb.
    Rejected for now; the `duckers-view/wayland` cargo feature keeps native Wayland, with the warning, for own builds.
14. **`max_show` counts per call site, in bind data.**
    DuckDB gives a scalar function no signal for the start of a query:
    the init callback's local state is created and dropped per thread and task, several times within one `GROUP BY`.
    Bind data exists once per planned call and is shared by its threads and executions,
    so the count is exact for a statement planned once and run once,
    which is every statement in the shell and every `execute`, `sql` and relation fetch in the Python client.
    Alternative: a process-wide count reset when no call is executing;
    rejected because it resets inside a single parallel query.
15. **The copy functions write one chart per file, with `std::fs`, to local paths.**
    A PNG or SVG file holds one image, so a file takes exactly one row, and `PARTITION_BY` makes one file per group;
    two rows for a file, zero rows and a `NULL` chart are errors rather than a silent pick.
    The v2 C API gives a copy function the file path as a string and no file-system handle
    (`FileSystem` is C++ only), so duckers writes the file itself and refuses remote schemes at bind time.
    Alternative: concatenate rows as `FORMAT blob` does, or name one file per row;
    rejected because the first writes files no viewer opens,
    and the second invents a naming scheme next to DuckDB's partitioning.
    Other C API constraints shape the details:
    - The C API has no way to declare the file extension DuckDB appends to partition files
      (the C++ `CopyFunction::extension`), so the name would be `data_0.`;
      duckers requires `FILE_EXTENSION` instead of writing a file under a name DuckDB does not know,
      which would escape DuckDB's cleanup on failure and its `RETURN_FILES` list.
    - The C API is batch-based: the batch callback runs on any thread
      and is handed the init data of the first file opened, not its own,
      so rows reach their file only in the flush callback, which DuckDB serializes per file.
      The file is rendered and written in finalize, once per file.
    - On failure DuckDB removes the files it saw finalized and the directories it created,
      so duckers only has to avoid partial files of its own: it writes nothing before the chart has rendered,
      and removes the file if the write fails.
16. **One run-time coordinate type for every axis.**
    The renderer draws on `Cartesian2d<AxisCoord, AxisCoord>`,
    where `AxisCoord` is an enum over plotters' coordinate types
    (`RangedCoordf64`, `RangedDate`, `RangedDateTime`, the segmented band axis, and `LogCoord`)
    with `f64` values: numbers as themselves, dates as days since the epoch, timestamps as microseconds since the epoch,
    bands as band positions.
    Each variant converts and delegates `map`, `key_points` and label formatting to the plotters coordinate, so labels,
    key points and pixels are plotters' own.
    Alternative: generic drawing code monomorphised per x × y coordinate pair.
    Rejected because every axis kind on either axis
    (horizontal histograms),
    more series (M5) and secondary axes (M6) multiply the combinations,
    while one `f64`-valued type keeps the drawing code free of axis kinds.
    Timestamps are exact as `f64` microseconds up to about the year 2255.
17. **A series has one column per axis, of any kind.**
    `SERIES` holds an x column and a y column, each numeric, integer buckets, dates, timestamps or categories,
    and the axis kinds, range checks and default extents are computed the same way for both axes.
    A vertical histogram has its buckets in x and the summed values in y; `histogram_horizontal` the other way round,
    so its buckets make the y axis a band axis exactly as `histogram_vertical`'s make the x axis one.
    Every series on a chart must agree on both kinds,
    and `y_range` is checked against the y kind as `x_range` is against the x kind
    (before any series, the kind is open on both axes).
    Alternative: numeric y values plus an orientation flag on histograms.
    Rejected because M5's horizontal error bars and boxplots and M6's secondary axes need buckets or times on y too.
18. **Log axes need positive bounds; they do not guess.**
    `x_log_scale`/`y_log_scale` apply to numeric axes only
    (checked at the call when the axis kind is known, at `draw_series` otherwise).
    A range bound that is zero or negative, explicit or from the data
    (including a histogram's baseline, 0 by default),
    is an error at render time that names where the bound came from and suggests `x_range`/`y_range`
    or `baseline`. plotters would replace a zero bound with `other_bound * 1e-5` and mirror negative ranges,
    which draws something, but rarely what was meant.
    With explicit positive bounds, points at or below zero are skipped
    and bars from a baseline of 0 start at the axis' low end.
    Without data a log axis shows `1..base`; a single value `v` is widened to `v / base .. v * base`.
    Labels use plotters' float printer with scientific notation
    (`1`, `100`, `1e6`)
    instead of `LogCoord`'s `{:?}` (`1000000.0`), and `x_label_formatter`/`y_label_formatter` apply as on a linear axis.
19. **Label formatters are a `format()` template or a strftime pattern, chosen by the text.**
    A formatter with a `{}` placeholder is a template: literal text around exactly one placeholder,
    `{{`/`}}` for braces, and an optional fmt spec `{:[[fill]align][sign][0][width][,][.precision][type]}` with `type`
    one of `f`, `e`, `g`, `d` (rounded to an integer) and `%`.
    `{}` without a spec is the axis' own label
    (plotters' text, e.g. `2.5`, the category name, a log decade), so `'{} °C'` only adds a unit.
    Anything else must contain a strftime conversion (`'%b %d'`).
    The text is checked when the formatter is set; whether it suits the axis is checked when the chart renders,
    since the axis kind may come from series drawn later: templates label numeric,
    log and integer-bucket axes (numbers) and category axes
    (fill, alignment, width and precision only),
    strftime patterns label date, timestamp and date-bucket axes; a mismatch is an error naming the axis kind.
    Alternative: DuckDB's full `format()` or `printf()`.
    Rejected because an extension cannot call them per label, and the fmt-spec subset covers what axis labels need.
    The last formatter call on an axis wins, as in plotters.

## Open questions

- Named parameters are verified on v2 C-API aggregates; scalar functions use the same signature API but were not
  tried (M0 spike 1).
- Whether duckdb#26109 gets fixed before v2.0 GA, which would also unblock `OVER ()` on series aggregates.
- macOS and Windows viewer behaviour is inferred from source, not tested;
  the README lists it as unverified and `docs/manual-acceptance-m3.md` has the checks to run.
- How the shim is enabled (`active_grammar_extensions` is per-connection) and how it ensures the core is loaded
  (M8).
- How far the grammar-extension API moves before v2.0 GA (M8).
- Grouped bars (`histogram_vertical` with `key`) need bar offsets plotters does not provide;
  left out until someone needs it.
