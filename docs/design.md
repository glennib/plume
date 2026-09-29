# plume design: plotters charts for DuckDB v2

plume lets SQL users draw charts with Rust's [`plotters`](https://github.com/plotters-rs/plotters) from inside DuckDB
v2, see them inline in the terminal or in a window, and write them as SVG or PNG.
Its API mirrors plotters, so a plotters user who knows the rules in [The plume paradigm](#the-plume-paradigm) can write
the SQL for a chart without a reference.
A `VISUALIZE` clause is optional sugar on top and comes last (see [The `VISUALIZE` clause](#the-visualize-clause)).

This document decides the user API and the behaviour.
The research behind it is in [`reports/`](reports/):

- [C API from Rust](reports/2026-09-27-duckdb-v2-c-api-from-rust.md) and
  [parser extensibility](reports/2026-09-27-duckdb-v2-parser-extensibility.md): the two-extension split.
- [v2 preview builds and C API details](reports/2026-09-27-duckdb-v2-preview-and-capi-details.md):
  what the v2 C API can and cannot do, verified with a probe extension.
- [plotters API inventory](reports/2026-09-27-plotters-api-inventory.md): every name the SQL API mirrors.
- [Native window display](reports/2026-09-27-native-window-display.md): how a chart reaches the user.
- [ggsql and prior art](reports/2026-09-27-ggsql-and-prior-art.md): what others chose.

## Goals and non-goals

Goals:

- Every 2D chart plotters can draw is reachable from SQL, in the same vocabulary.
- Output as SVG (`VARCHAR`) and PNG (`BLOB`), files through `COPY`, and a `show()` that opens a viewer.
- The extension is built once per platform against the stable v2 C API and keeps loading across DuckDB releases.

Non-goals:

- A grammar-of-graphics layer (ggsql, ggplot).
  SQL does the statistics and reshaping; plume only draws rows.
- Interactivity (zoom, hover). plotters is a static renderer.
- 3D charts. plotters supports them and they can be added later, but they are not planned.
- Theming systems beyond what plotters exposes.

## The plume paradigm

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
   sized label areas, a mesh call, a series style and a legend call. plume infers ranges from the data,
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
| `CHART` | `ChartBuilder` + `ChartContext` + the root `DrawingArea`, or a grid, titled area or `Pie` on it | `chart()`, `split_evenly`, `titled`, `pie` | outputs, `configure_*`, `draw_series`, the layout functions |
| `MESH` | `MeshStyle`, or `SecondaryMeshStyle` | `configure_mesh(chart)`, `configure_secondary_axes(chart)` | `draw` |
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
| `error_bar_vertical(x, min, avg, max)` / `error_bar_horizontal(y, min, avg, max)` | `ErrorBar::new_vertical` / `new_horizontal` per row | One element per row; the key on x (vertical) or y (horizontal). |
| `candle_stick(x, open, high, low, close)` | `CandleStick::new(...)` per row | |
| `boxplot_vertical(bucket, value)` / `boxplot_horizontal(bucket, value)` | `Boxplot::new_vertical(key, &Quartiles::new(values))` | One box per distinct bucket (plotters' `key`, decision 21), with plotters' `Quartiles` computed per bucket. |

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
so plume declares its aggregates order-independent, and the planner then drops such an `ORDER BY` silently.
The same bug crashes `series_agg(...) OVER ()`; window frames are unsupported until the fix lands.

Argument types:

- `x`: any numeric type (continuous axis, mapped to `DOUBLE`), `DATE`, `TIMESTAMP`, `TIMESTAMPTZ` and the
  `TIMESTAMP_*` variants (time axis), or `VARCHAR` (category axis).
- The `y` of `error_bar_horizontal` is its key, of the same types as `x`.
- `y`, `value`, `min`, `avg`, `max`, `open`, `high`, `low`, `close`: any numeric type.
- `bucket`: `VARCHAR` (one band per category), any integer type (a segmented integer axis from min to max), or `DATE`
  (one band per day).
  A histogram's buckets may also be `DOUBLE`, `FLOAT` or `DECIMAL`, which `.step(s)` bins into bands
  (drawing them without a step is an error that names `.step(s)`); a boxplot's may not.
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
| `filled()` | `ShapeStyle::filled` | point markers, line markers, histogram, candle bodies, error-bar dots; accepted on `area_series`, whose polygon is always filled | histogram and area filled, others not |
| `label(text)` | `SeriesAnno::label` | all | none (series not in legend) |
| `point_size(px)` | `LineSeries::point_size` | `line_series` | 0 |
| `size(px)` | the `size` argument of `PointSeries::new` | `point_series` | 3 |
| `marker(name)` | the element type parameter of `PointSeries::new` (`'circle'`, `'cross'`, `'triangle'`, `'pixel'`) | `point_series` | `'circle'` |
| `margin(px)` | `Histogram::margin` | histograms | 5 |
| `baseline(v)` | `Histogram::baseline`, the `baseline` argument of `AreaSeries::new` | histograms, `area_series` | 0 |
| `border_style(color [, stroke_width])` | `AreaSeries::border_style` | `area_series` | transparent |
| `size(px)`, `spacing(px)` | the `size` and `spacing` arguments of `DashedLineSeries::new` | `dashed_line_series` | 5, 5 |
| `width(px)` | the `width` argument of `ErrorBar`, `CandleStick`, `Boxplot::width` | error bars, candles, boxplots | 10 |
| `gain_style(color)`, `loss_style(color)` | `CandleStick::new` arguments; the series' stroke width and `filled()` apply to both, and `style()` is an error | `candle_stick` | `'green'`, `'red'` |
| `step(s)` | `(lo..hi).step(s).use_round().into_segmented()` for the bucket range (decision 22) | histograms with numeric or integer buckets | none |

The legend glyph for a labelled series is derived from its kind: a short line
(with a marker if `point_size` is set),
a marker, a filled rectangle (histograms and areas), a short dashed line, or a small error bar,
candle or box. plotters requires a closure here (`SeriesAnno::legend`); there is no SQL equivalent, and none is planned.

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
| `x_log_scale([base])`, `y_log_scale([base])` | `(lo..hi).log_scale().base(b)`; numeric axes only, positive bounds (decision 18) | linear; base 10 |
| `x_monthly()`, `y_monthly()`, `x_yearly()`, `y_yearly()` | `(lo..hi).monthly()`, `.yearly()` (`IntoMonthly`, `IntoYearly`): key points at month or year starts; date and timestamp axes only (decision 23) | the coordinate's key points |
| `root_fill(color)` | `root.fill(&WHITE)` | `'white'` (plotters: black bitmap, transparent SVG) |
| `draw_series(series \| series[])` | `ChartContext::draw_series` | |
| `configure_mesh()` | `ChartContext::configure_mesh()` | returns `MESH` |
| `configure_series_labels()` | `ChartContext::configure_series_labels()` | returns `SERIES_LABELS` |
| `set_secondary_coord()` | `ChartContext::set_secondary_coord(x, y)` with ranges from the secondary series | implied by the three below (decision 25) |
| `secondary_x_range(lo, hi)`, `secondary_y_range(lo, hi)` | the range arguments of `set_secondary_coord` | data extent of the secondary series; the primary x axis when the secondary x has neither series nor range |
| `draw_secondary_series(series \| series[])` | `DualCoordChartContext::draw_secondary_series` | |
| `configure_secondary_axes()` | `configure_secondary_axes()` | returns `MESH` (mesh lines disabled, as in plotters); drawn with defaults right after the primary mesh if the chain never draws it |

The secondary x axis must have the primary x axis' kind; the secondary y axis may have any kind.
The secondary axes are linear (or time, band and category axes by the data); there are no secondary scale calls.
On a chart with secondary axes the right label area defaults to 40 px,
and the top one too when the secondary x axis differs from the primary one, unless the chain sets them.
Secondary series continue the `Palette99` numbering of the primary ones and share their legend.
These methods, like every other in this table except `root_fill`, apply to a cartesian `CHART` only
(see [Layout](#layout)).

`build_cartesian_2d` itself has no SQL function.
The range expressions it takes are covered by `x_range`/`y_range`, the log,
monthly and yearly combinators by `x_log_scale`/`y_log_scale`, `x_monthly`/`x_yearly` and the `y_` ones,
the segmented and stepped combinators by the series kinds and `step(s)`, and the value types by the column types.
The last scale call on an axis wins.

### Mesh

`configure_mesh(chart)` returns a `MESH`; every method below returns `MESH`; `draw()` returns the `CHART`.
Names, parameters and defaults are plotters' (`MeshStyle`, plotters 0.3.7).

| SQL | Parameters | Default |
|---|---|---|
| `x_desc(text)`, `y_desc(text)` | `VARCHAR` | none |
| `axis_desc_style(size \| font)` | | the label style |
| `x_labels(n)`, `y_labels(n)` | max labels and bold lines, 0 to 1000 | 11 |
| `x_label_formatter(fmt)`, `y_label_formatter(fmt)` | a DuckDB `format()` template for numbers and categories (`'{:.1f} °C'`), a `strftime` pattern for date and time axes (`'%b %d'`); see decision 19 | the coordinate's formatter |
| `label_style(size \| font)`, `x_label_style(...)`, `y_label_style(...)` | | sans-serif 12, black |
| `x_label_offset(px)`, `y_label_offset(px)` | may be negative | 0 |
| `x_max_light_lines(n)`, `y_max_light_lines(n)`, `max_light_lines(n)` | 0 to 100 | 10 |
| `light_line_style(color [, w])`, `bold_line_style(color [, w])`, `axis_style(color [, w])` | | `mix('black', 0.1)`, `mix('black', 0.2)`, `'black'` |
| `disable_x_mesh()`, `disable_y_mesh()`, `disable_mesh()` | | mesh drawn |
| `disable_x_axis()`, `disable_y_axis()`, `disable_axes()` | | axes drawn |
| `set_tick_mark_size(position, px)`, `set_all_tick_mark_size(px)` | position `'top'`, `'bottom'`, `'left'`, `'right'`; a negative size points inwards | 5 |
| `draw()` | | returns `CHART` |

If a chain never calls `configure_mesh().draw()`,
plume draws `configure_mesh().draw()` with defaults before the first series.
A chain that does call it controls the order, as in plotters.

On the `MESH` of `configure_secondary_axes()` only `SecondaryMeshStyle`'s setters apply: `axis_style`, `x_label_offset`,
`y_label_offset`, `x_labels`, `y_labels`, `x_label_formatter`, `y_label_formatter`, `axis_desc_style`, `x_desc`,
`y_desc`, `label_style`, `set_all_tick_mark_size`, `set_tick_mark_size` and `draw`.
The others are errors naming `SecondaryMeshStyle`.

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
plume draws `configure_series_labels().draw()` with defaults last.

### Layout

A `CHART` is one of four roots: a cartesian chart (`chart()`), a grid, a titled area or a pie.

| SQL | plotters | Notes |
|---|---|---|
| `split_evenly(charts, rows, cols)` | `root.split_evenly((rows, cols))`, one chart drawn per cell | `charts` is a `CHART[]`, row-major; `NULL` elements and missing cells are blank; more charts than cells is an error; 1 to 64 rows and columns; cells may be any root |
| `titled(chart, text [, size \| font])` | `root.titled(text, style)` | sans-serif 20 (plotters has no default) |
| `pie(size, label)` | `Pie::new(center, radius, sizes, colors, labels)` | an aggregate; one slice per row, colours from `Palette99`; `order_by :=` (default: label, then size); rows with a `NULL`, `NaN`, infinite or non-positive size skipped; `label` shown as `::VARCHAR` |
| `start_angle(deg)` | `Pie::start_angle` | 0 (three o'clock) |
| `label_style(size \| font)` | `Pie::label_style` | sans-serif at 5% of the radius |
| `percentages(size \| font)` | `Pie::percentages` | none |
| `label_offset(px)` | `Pie::label_offset` | 5% of the radius |
| `radius(px)` | the `radius` argument of `Pie::new` | the largest radius that keeps the labels inside the area |

`root_fill`, `titled`, `split_evenly`, the outputs, `show` and the copy functions apply to every root;
each root fills its own area, so a blank grid cell shows the grid's fill.
The builder methods of [Chart builder](#chart-builder) are errors on the other roots, naming the method and the root;
`caption` on them points at `titled`.
The pie methods are errors on the other roots.

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
  The embedded font is registered under every family any `FONT` in the chart names.
  It is one regular face, and plotters falls back to a family's normal face, so bitmaps draw bold,
  italic and oblique text in the regular face; SVG writes the style for the viewer.
- **Sizes** are integer pixels.
  plotters' relative sizes (`10.percent()`) are not exposed.

### Coordinates and ranges

- The column types pick the axes: numeric → `RangedCoordf64`; `DATE` → `RangedDate`;
  `TIMESTAMP` and friends → `RangedDateTime`; `VARCHAR` → a category axis
  (`RangedSlice`, segmented for histograms);
  integer histogram and boxplot buckets → a segmented integer axis;
  numeric histogram buckets with `.step(s)` → one band per bin of width `s`.
  The x axis follows `x` or a vertical histogram's buckets, the y axis `y` or a horizontal histogram's buckets
  (decision 16).
  Every series drawn on one chart must agree on the x kind and on the y kind; a mismatch is an error naming both.
- Default ranges are the data extent over all series on the axis, with no padding (plotters' `fitting_range`).
  Histograms and area series include their baseline, error bars their minima and maxima,
  candlesticks their lows and highs, and boxplots their fences.
  An empty chart gets `0..1`.
- `x_range(lo, hi)` with `lo > hi` reverses the axis, as in plotters.
  `lo = hi` is an error.
  The types must match the axis kind (`x_range(DATE '2024-01-01', DATE '2024-02-01')` on a time axis).
- Out-of-range points are clamped to the plotting rectangle, not clipped.
  That is plotters' behaviour and plume keeps it.
- Category axes list categories in `order_by` order (default: sorted).

### Output and display

| SQL | plotters | Result |
|---|---|---|
| `to_svg(chart [, width, height])` | `SVGBackend::with_string` | `VARCHAR` |
| `to_png(chart [, width, height])` | `BitMapBackend::with_buffer` + PNG encoding | `BLOB` |
| `show(chart, viewer := ..., wait := ..., width := ..., height := ...)` | | shows the chart, see [Display](#display); returns the `CHART` |
| `plume_set(key, value)`, `plume_get(key)` | | set or read a process-wide `show()` default, see [Display](#display); `VARCHAR` |
| `COPY (SELECT chart ...) TO 'f.png' (FORMAT png, WIDTH w, HEIGHT h)`, `(FORMAT svg)` | `BitMapBackend::new(path)`, `SVGBackend::new(path)` | one file per chart, one chart per file |

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
  (`PLUME_VIEWER`, `PLUME_WAIT`)
  and can be changed in a session with `plume_set('viewer', 'browser')`, which returns the new value;
  `plume_get('viewer')` reads one.
  They are process-wide because a v2 C-API extension cannot register `SET` options.
  The grammar shim registers `plume_viewer`, `plume_wait` and `plume_max_show` as real options,
  which `show()` reads through its context when they exist:
  a value set with `SET` takes precedence over the process-wide default, and the named parameter over both.
  Their default is `NULL`, meaning the process-wide setting, so loading the shim changes nothing by itself.
  An invalid environment value is an error from every `show()` that needs it, until `plume_set` replaces it.
- A named argument left out or passed as `NULL` takes its default (the setting, or 640×480);
  only a `NULL` chart gives `NULL`.
- `show()` is volatile, so the optimizer neither folds it at plan time nor caches it.
  A multi-row result shows every row, capped by `plume_set('max_show', n)` (default 10; `0` shows nothing).
  The cap counts per `show()` call in a planned statement, in the call's bind data:
  every statement the shell or a client plans starts at zero,
  and a prepared statement keeps one count across its executions.
  Rows past the cap are returned unchanged.
- A subprocess viewer (a helper binary owning its own main thread) is the only route to a detached native window on
  macOS.
  It is deliberately not planned: shipping a second binary inside a `.duckdb_extension` runs into Apple Silicon signing
  and managed-Windows application control, and the browser covers the case.
  A user-installed `plume-view` on `PATH` could be added as a fourth viewer without changing the API.

### The `VISUALIZE` clause

The grammar shim `plume_visualize` adds a statement suffix to DuckDB's parser:

```sql
<query> VISUALIZE <target list>
```

which is `SELECT <target list> FROM (<query>)`; `VISUALISE` is the same word.
The target list is one or more expressions with optional aliases, usually one chart expression,
and the query is any `SELECT` (with CTEs, `ORDER BY`, `LIMIT`, set operations).
`GROUP BY` belongs to the query; the series aggregates in the chart expression consume the query's rows.

- It applies to a statement, so `EXPLAIN` and `PREPARE` take it, and a subquery, a view body or `COPY (...)` does not.
- It is active per connection: `SET active_grammar_extensions = ['plume_visualize']` switches it on
  (`RESET` off), which is DuckDB's rule for every grammar extension.
  While it is active, `visualize` and `visualise` are reserved words: fine after `AS`, quoted as identifiers.
- Loading the shim loads the core when it is not loaded yet: the `plume.duckdb_extension` next to the shim's own
  file, or the installed extension of that name; without either the load fails.
- The shim also registers the `plume_viewer`, `plume_wait` and `plume_max_show` settings
  (see [Display](#display)).

```sql
SELECT day, temp FROM weather WHERE city = 'Oslo'
VISUALIZE chart().caption('Oslo').draw_series(line_series(day, temp)).show();
```

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

The same chart in plume:

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
  A `CHART` from an older plume with an incompatible format fails to decode with a clear message.
  Persisting chart values in tables is allowed but not a stability promise before 1.0.
- **Threads.**
  Rendering is pure and runs on whichever thread DuckDB uses.
  Only `show()` and the copy function have side effects.
  The calling thread is unpredictable unless `threads = 1`, which only matters for macOS windows.

## Architecture

Two extensions, as before, but with the weight moved to the core.

1. **Core (Rust, stable C ABI v2).**
   - `plume-sys`: bindgen over a vendored `duckdb_extension_v2.h` pinned to C API `v2.0.0`.
     The indirection macros do not survive bindgen,
     so the crate stores the function-pointer table returned by `get_api` and calls through it.
     No published Rust crate covers loadable v2 extensions today
     (`libduckdb-sys`'s `capi-v2` binds `duckdb_v2.h` only; `duckdb-neo` is unreleased and links the symbols directly).
   - `plume`: the `cdylib` with the entrypoint `plume_init_c_api_v2`, the value types and their casts,
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
   A v2 C++ extension links DuckDB statically and loads only into the DuckDB version it was built from
   (decision 29),
   so the shim is built inside DuckDB's own CMake build against the pinned source
   (`make shim`, a few minutes of compiling DuckDB) and lives in `shim/`.
   It also registers the `plume_*` settings and loads the core.

Prior art shaped a few choices: usql's `\chart` shows that inline terminal images work in a SQL shell,
ggsql-duckdb shows a browser viewer and an output-mode setting,
and anofox's SVG-returning functions show that values in the result grid are enough for many uses.

## Not yet done

- Distribution: the core built once per platform, the shim per DuckDB version.
  Until the community-extensions repository deploys its v2 leg
  ([community-extensions#2723](https://github.com/duckdb/community-extensions/issues/2723)), releases are GitHub assets
  loaded with `-unsigned`; the submission follows when the leg exists.
- Secondary scale calls (log, monthly, yearly), `Pie::donut_hole` and per-slice colours are not covered.

## Decisions

Each item records the choice, the alternative, and why.

1. **Function API first, `VISUALIZE` last.**
   The first draft of this design led with the clause.
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
    and `plume_set` plus environment variables cover session defaults until the shim can register real options.
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
    Rejected for now; the `plume-view/wayland` cargo feature keeps native Wayland, with the warning, for own builds.
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
    (`FileSystem` is C++ only), so plume writes the file itself and refuses remote schemes at bind time.
    Alternative: concatenate rows as `FORMAT blob` does, or name one file per row;
    rejected because the first writes files no viewer opens,
    and the second invents a naming scheme next to DuckDB's partitioning.
    Other C API constraints shape the details:
    - The C API has no way to declare the file extension DuckDB appends to partition files
      (the C++ `CopyFunction::extension`), so the name would be `data_0.`;
      plume requires `FILE_EXTENSION` instead of writing a file under a name DuckDB does not know,
      which would escape DuckDB's cleanup on failure and its `RETURN_FILES` list.
    - The C API is batch-based: the batch callback runs on any thread
      and is handed the init data of the first file opened, not its own,
      so rows reach their file only in the flush callback, which DuckDB serializes per file.
      The file is rendered and written in finalize, once per file.
    - On failure DuckDB removes the files it saw finalized and the directories it created,
      so plume only has to avoid partial files of its own: it writes nothing before the chart has rendered,
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
    the other series kinds and secondary axes multiply the combinations,
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
    Rejected because horizontal error bars and boxplots and secondary axes need buckets or times on y too.
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
20. **Extra per-row values live in the kind's options.**
    A `SERIES` keeps one column per axis
    (decision 17);
    the value column of an error bar is its `avg`, of a candlestick its `close`, of a boxplot its median,
    and the other values (min and max; open, high and low; the five quartile values) are vectors in the kind's options,
    one entry per point.
    The default extent of the value axis includes them.
    A boxplot stores plotters' `Quartiles::values()` per bucket, the `f32` numbers widened to `f64`,
    computed at finalize.
    Fed back through `Quartiles::new`, those five give the same quartiles exactly
    (the 25th, 50th and 75th percentile of five sorted values are the middle three),
    but fences only within one `f32` rounding step, since plotters recomputes them from the rounded quartiles.
    So the renderer draws the stored numbers: `Boxplot` is built as plotters builds it,
    and its drawing code is handed the positions of the stored values, which also avoids its `f32` coordinates,
    which the chart's `f64` axes cannot map.
    Alternative: the raw values per bucket, with the quartiles computed at render time.
    Rejected because a boxplot of a million rows would carry the million rows.
21. **A boxplot's first parameter is `bucket`, not `key`.** plotters calls it `key`
    (`Boxplot::new_vertical(key, &quartiles)`),
    but every series aggregate has a named `key :=` parameter, and DuckDB refuses two parameters of one name.
    `bucket` is the histogram's word for the same thing: the values that make a band axis.
22. **`step(s)` bins at multiples of `s`, and integer buckets can be stepped.**
    Bin `k` holds the values from `k * s` up to `(k + 1) * s`
    (`floor(x / s)`, with a quotient within a relative 1e-9 of an integer taken as that integer,
    so `0.3` is in bin 3 of step `0.1`), and the bands run from the lowest bin to the highest,
    or over the bins that cover `x_range(lo, hi)`.
    A band is labelled with its bin's start at its centre, through the numeric label path,
    so `x_label_formatter` templates apply. plotters' `(lo..hi).step(s).use_round()` places grid values from `lo`
    and maps a value to the nearest one; with the data's minimum as `lo`,
    that would make the bins depend on the data and put them off the round numbers.
    Integer buckets become numbers when stepped
    (`histogram_vertical(age, 1).step(10)`), since `(0..100).step(10)` is the same combinator.
    A line or other series with numeric x on a stepped axis is placed at the centre of its value's bin,
    as points are on date bands.
    Two histograms on one axis must have the same step, and a stepped axis has no log scale.
23. **Monthly and yearly key points are scales.**
    `x_monthly()` and friends are appended `Scale` variants, checked against the axis kind as the log scale is.
    On date and timestamp axes the key points and labels are plotters' `Monthly`/`Yearly` ones
    (`2024-1`, the month unpadded, which is plotters' `{}-{}` format);
    a strftime formatter replaces the labels as on any time axis. plotters has no `Monthly` over `NaiveDateTime` ranges,
    so timestamps use `DateTime<Utc>`, which is how plume reads them.
    On a date-bucket axis the bands stay one per day,
    and the key points and labels go on the bands of the days that start a month
    or year. plotters' `(lo..hi).monthly().into_segmented()` would make one band per month instead,
    but its `Histogram` does not sum the days of a month
    (it draws one overlapping bar per distinct day),
    so that is not a monthly histogram either; SQL's `date_trunc('month', d)` is the way to bin by month.
24. **Styles of the area, dashed line, error bar, candlestick and boxplot kinds follow the paradigm's defaults, with
    plotters' shapes.**
    Series colours come from `Palette99` for every kind, including boxplots (plotters' default is black) and areas
    (plotters' examples use a translucent colour; `mix` gives one).
    A candlestick has no `style`: `CandleStick::new` takes a gain and a loss style,
    so `style()` is an error naming `gain_style` and `loss_style`,
    which default to `'green'` and `'red'` and take the series' stroke width and fill.
    An area's polygon is always filled, so `filled()` is accepted on it and changes nothing;
    a dashed line and a boxplot have nothing `filled()` could fill,
    so it is an error there. plotters draws a boxplot's box unfilled
    and its lower whisker 1 px wide whatever the stroke width; plume keeps that.
    `Boxplot::whisker_width` and `offset` are not exposed.
25. **Secondary axes are implied, inferred and defaulted.**
    `draw_secondary_series`, `secondary_x_range`/`secondary_y_range`
    and `configure_secondary_axes` imply `set_secondary_coord()`, which takes no ranges:
    plotters draws nothing on secondary axes it was not given (rule 8).
    The secondary ranges are the extent of the secondary series,
    and a secondary x axis with neither series nor range is the primary x axis.
    A chart with secondary axes gets them drawn with defaults right after the primary mesh
    (whether or not a secondary series is drawn yet),
    and 40 px right and, when the secondary x axis differs, top label areas unless the chain sizes them;
    the top and right sizes are therefore `Option`s in the spec, `None` until set.
    The secondary x axis must have the primary's kind,
    checked by whichever call draws the second of the two and at render time. plotters allows any pair,
    but a dual-coordinate chart shares its x data in practice, and a mismatch is almost always a mistake.
    The secondary axes have no scale calls; `secondary_x_log_scale` and friends can be appended later.
    Alternative: make `set_secondary_coord(x_lo, x_hi, y_lo, y_hi)` take the ranges as plotters does.
    Rejected for the same reason as decision 10.
26. **A `CHART` is a root; `Chart` stays the cartesian builder.**
    The value behind `CHART` is an enum of a cartesian chart, a grid, a titled area and a pie,
    so the layouts compose with every output function and with each other without new SQL types.
    A method of `ChartBuilder` or `ChartContext` on another root is an error naming the method and the root kind,
    rather than reaching into a cell; `root_fill`, `titled` and `split_evenly` work on every root.
    Every root fills its own area; `titled` copies the fill of the chart it wraps so the title strip matches it.
    Alternative: a separate `LAYOUT` type.
    Rejected because `show`, `to_png` and `COPY` would all need a second overload,
    and a grid cell could not hold a grid.
27. **`split_evenly` takes a list; `titled` takes a chart.** plotters splits an area and draws into the parts;
    in SQL the charts exist first, so `split_evenly(charts, rows, cols)` takes the `CHART[]`
    and fills the cells row-major, with `NULL` elements and missing cells blank and more charts than cells an error.
    Rows and columns are capped at 64, since a cell of a 8192 px image is then at least 128 px.
    `titled(chart, text)` takes the chart
    (an earlier draft wrote `titled(text)`),
    with sans-serif 20 when no style is given: plotters' `titled` has no default.
28. **A pie is an aggregate over rows, ordered like a series, and fits its labels.**
    `pie(size, label)` makes one slice per row, as `Pie::new` takes parallel slices;
    duplicate labels are separate slices (SQL aggregates them if wanted).
    Slices follow `order_by`, then the label in its type's order, then the size,
    so the result does not depend on row order.
    Colours are `Palette99` in slice order, the series rule.
    `Pie` draws on the backend directly, so its labels are not clipped to the area:
    without `radius(px)` plume picks the largest radius at which every label, placed as `Pie` places it,
    stays 10 px inside the area.
    `Pie::donut_hole` and per-slice colours are left out until someone needs them.
29. **The shim is built inside DuckDB's build and links DuckDB statically.**
    A v2 loadable C++ extension is built with `EXTENSION_STATIC_BUILD`:
    DuckDB itself is linked into the extension and its symbols hidden,
    and the release binaries export nothing for an extension to resolve against
    (the preview CLI has no `duckdb::` symbol in its dynamic table).
    So the shim cannot be compiled against headers alone; `make shim` fetches the pinned source
    (one commit, sparse and without unneeded blobs, into `.duckdb/<version>/src`),
    configures DuckDB's CMake build with the shim as an out-of-tree extension config
    and DuckDB's `EXTENSION` optimisation profile
    (the baseline its distributed extensions are built for),
    and builds only the loadable target, without the shell, the tests, parquet and jemalloc.
    The result loads only into that exact DuckDB version, which is DuckDB's rule for the C++ ABI.
    Alternative: link against the shared `libduckdb` from the release tarball;
    rejected because the file would then need that library next to it at run time, and would carry a second,
    complete copy of DuckDB either way.
30. **`VISUALIZE` replaces the `SelectStatement` rule and takes a target list.**
    The rule becomes `SelectStatement <- SelectStatementInternal VisualizeClause?`,
    with `VisualizeClause <- VisualizeKeyword TargetList`, and the shim's transform builds the wrapping `SELECT`
    (or passes the statement through when the clause is absent).
    One parse, no backtracking, and the suffix comes after `ORDER BY` and `LIMIT`, where ggsql puts it.
    `SelectStatement` is only referenced from the statement list, so the clause is a statement suffix:
    `PREPARE` takes a statement and so takes it; `EXPLAIN` has a select rule of its own, replaced the same way;
    a subquery, a view body and `COPY (...)` use `SelectStatementInternal` and do not.
    A target list rather than one expression costs nothing and allows an alias or a second column.
    Both words are added to `ReservedKeyword`, as DuckDB's own demo does for `EXTEND`:
    otherwise `SELECT x VISUALIZE ...` would read `VISUALIZE` as the alias of `x`,
    and `FROM t VISUALIZE ...` as the alias of `t`.
    Alternative: prepend a choice (`VisualizeStatement / SelectStatementInternal`);
    rejected because it parses every `SELECT` twice when the clause is absent.
31. **The shim does not switch its grammar on; the connection does.**
    `active_grammar_extensions` is a local setting by DuckDB's design, and the C++ entrypoint sees the database,
    not the connection that ran `LOAD`.
    An `OnConnectionOpened` callback could activate it for connections opened later, but not for the one that loaded it,
    which is the only one the shell has; a rule that holds for some connections and not others was rejected.
    `SET active_grammar_extensions = ['plume_visualize']` (in `~/.duckdbrc` for the shell) is the way.
    The shim does load the core, which is a database-wide act: the `plume.duckdb_extension` next to its own file
    (found with `dladdr`, or the module handle on Windows),
    else the installed extension, else the load fails with both places named,
    since a shim without the core would fail every query it accepts.
32. **Session settings default to `NULL` and override the process-wide ones.**
    `plume_viewer`, `plume_wait` and `plume_max_show` are extension options of the shim, so `SET`, `SET GLOBAL`,
    `RESET` and `current_setting` work on them, and `duckdb_settings()` lists them.
    `show()` reads them through its context when it is bound
    (the C API reads any option by name)
    and takes them over `plume_set` and the environment; a `NULL`
    (the default) means the process-wide setting, so loading the shim changes nothing until a `SET`.
    DuckDB stores nothing for an option registered with a `NULL` default,
    and `current_setting` then calls it unrecognized,
    so the shim stores a typed `NULL` right after registering each one.
    `plume_set` and `plume_get` keep their process-wide meaning, since they also exist without the shim.
    Alternative: make the `SET` callback write through to the process-wide setting;
    rejected because the callback runs in the C++ copy of DuckDB and cannot reach the core's state.

## Open questions

- Named parameters are verified on v2 C-API aggregates; scalar functions use the same signature API but were not
  tried (the first spike in [`reports/2026-09-27-spikes.md`](reports/2026-09-27-spikes.md)).
- Whether duckdb#26109 gets fixed before v2.0 GA, which would also unblock `OVER ()` on series aggregates.
- macOS and Windows viewer behaviour is inferred from source, not tested;
  the README lists it as unverified and [`manual-acceptance-show.md`](manual-acceptance-show.md) has the checks to run.
- How far the grammar-extension API moves before v2.0 GA: the shim is built against the pinned preview and is
  rebuilt with each pin.
- Grouped bars (`histogram_vertical` with `key`) need bar offsets plotters does not provide;
  left out until someone needs it.
