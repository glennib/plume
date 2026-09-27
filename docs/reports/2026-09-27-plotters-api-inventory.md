# plotters 0.3.7 public API inventory

Research date: 2026-09-27.
Sources inspected:

- `plotters`, `plotters-backend`, `plotters-bitmap` and `plotters-svg` 0.3.7 from the local cargo registry;
- `plotters-rs/plotters` branch `master`, HEAD `c63248e` (2026-03-17);
- the `font-kit` 0.14.2 manifest and `src/source.rs`.

The latest release on crates.io is 0.3.7, published 2024-09-08.
Items marked *(verified)* were checked with a probe program built against 0.3.7
(SVG and bitmap backends, with and without `ab_glyph`).

## Summary

plotters is a drawing library with a chart layer on top, not a declarative plotting library.
A chart is built imperatively: backend → root `DrawingArea` → `ChartBuilder` → `ChartContext`,
then `configure_mesh().draw()`, one `draw_series(...)` per series, and optionally `configure_series_labels().draw()`.
Everything is drawn immediately, in call order.

- Axis ranges are always given by the caller.
  The library never fits a range to the data.
  The only helper is `plotters::data::fitting_range`, which returns `min..max` of an iterator.
- A "series" is any iterator of elements.
  The series types (`LineSeries`, `AreaSeries`, `Histogram`, `PointSeries`, `SurfaceSeries`, ...) are iterator adaptors.
  `ErrorBar`, `Boxplot` and `CandleStick` are elements, drawn by mapping data rows to elements.
- Points are drawn in the order given.
  `Histogram::data` is the only place that aggregates: it **sums** the y values per discrete x bucket.
- Values outside the axis range are not clipped.
  Each element vertex is clamped to the plotting rectangle, so lines bend onto the border.
- Sizes are in backend pixels (with optional relative sizes).
  There is no DPI setting.
- Many options take closures: label formatters, legend glyphs, custom point markers, `Histogram::style_func`,
  `SurfaceSeries` (the surface itself is a function), 3D projection, nested coordinates and key-point functions.
  The full list is in [Closure-shaped parameters](#closure-shaped-parameters).
- Text in bitmaps needs a font engine: `ttf`
  (default; system fonts through font-kit, fontconfig and FreeType on Linux)
  or `ab_glyph` (pure Rust, fonts registered from `&'static [u8]`).
  Without either, drawing bitmap text panics.
  SVG text is rendered by the viewer, but its layout still uses the font engine for measurements.
- `master` has 54 commits since v0.3.7 but no release.
  The relevant changes are float formatting for custom key points, the meaning of a negative tick size,
  HSL hue wrapping, a `serialization` feature and dependency bumps
  (see [Changes on master since 0.3.7](#changes-on-master-since-037)).

## Conventions used below

- File paths are relative to the crate root, `plotters-0.3.7/` unless another crate is named.
- "px" means backend pixels.
  "Data" means values in the chart's coordinate system (guest coordinates).
- `SizeDesc` parameters accept `i32`, `u32`, `f32`, `f64` (pixels) or a relative size
  (`n.percent()`, `n.percent_width()`, `n.percent_height()`, optionally `.min(px)` / `.max(px)`),
  see [Styles](#8-styles).
- `ChartBuilder`, `MeshStyle`, `SecondaryMeshStyle`,
  `SeriesLabelStyle` and `Axes3dStyle` setters take `&mut self` and return `&mut Self`.
  Series and element builders (`LineSeries::point_size`, `AreaSeries::border_style`, `Histogram::*`, `Boxplot::*`) take
  and return `self`.
- Every drawing call returns `Result<_, DrawingAreaErrorKind<DB::ErrorType>>`
  (`BackendError(DrawingErrorKind)`, `SharingError`, `LayoutError`).
  `DrawingErrorKind` is `DrawingError(E)` or `FontError(Box<dyn Error>)`.
  The prelude aliases this as `DrawResult<T, DB>`.

## 1. Backends and output

Sources: `plotters-backend-0.3.7/src/lib.rs`, `plotters-svg-0.3.7/src/svg.rs`, `plotters-bitmap-0.3.7/src/bitmap.rs`,
`plotters-bitmap-0.3.7/src/bitmap/target.rs`, `plotters-bitmap-0.3.7/src/gif_support.rs`,
`plotters-bitmap-0.3.7/src/bitmap_pixel/*.rs`, `src/style/font/*.rs`, `src/lib.rs`.

### The `DrawingBackend` trait

Backends implement `plotters_backend::DrawingBackend`.
Only `get_size`, `ensure_prepared`, `present` and `draw_pixel` are required.
The others (`draw_line`, `draw_rect`, `draw_path`, `draw_circle`, `fill_polygon`, `draw_text`, `estimate_text_size`,
`blit_bitmap`) have default implementations on top of a CPU rasterizer in `plotters_backend::rasterizer`.
Backend coordinates are `(i32, i32)` with `(0, 0)` at the top left.

### `SVGBackend` (feature `svg_backend`)

| Constructor | Parameters | Output |
|---|---|---|
| `SVGBackend::new(path, size)` | `path: &'a T` where `T: AsRef<Path> + ?Sized`; `size: (u32, u32)` | Builds the document in memory and writes the file on `present()` or drop. |
| `SVGBackend::with_string(buf, size)` | `buf: &'a mut String`; `size: (u32, u32)` | Appends the document to `buf`. The closing tags are written on `present()` or drop. |

- The root element is `<svg width="W" height="H" viewBox="0 0 W H" xmlns=...>`, with unitless (px) width and height.
- Elements emitted: `rect`, `line`, `polyline`, `polygon`, `circle`, `text` and, if enabled, `image`.
  Colours are written as `fill`/`stroke="#RRGGBB"` plus an `opacity` attribute.
  Anything with alpha 0 is skipped, so an unfilled SVG background is transparent.
- Text is written as `<text x y dy text-anchor font-family font-size opacity fill>` with the text XML-escaped:
  - `font-family` is `FontFamily::as_str()`, e.g. `sans-serif` or the given name.
    No font data is embedded or referenced.
  - `font-size` is the plotters font size divided by 1.24 (a size-12 label is written as `9.677...`) *(verified)*.
  - `FontStyle::Bold` becomes `font-weight="bold"`.
    Italic and oblique become `font-style`.
  - The anchor is `text-anchor` (`start`/`middle`/`end`) plus `dy` (`0.76em` top, `0.5ex` centre, `-0.5ex` bottom).
  - Rotation is `transform="rotate(90|180|270, x, y)"`.
- Text layout (label alignment, legend box size, `titled`, `Pie` labels) still calls the font engine's
  `estimate_layout`, because `SVGBackend` does not override `estimate_text_size`.
  With `ab_glyph` this fails with `FontUnavailable` until a font is registered, even for SVG output *(verified)*.
- `blit_bitmap` (used by `BitMapElement`) writes an `<image href="data:png;base64,...">` only
  when plotters-svg's optional `image` dependency is enabled
  (feature `plotters-svg/bitmap_encoder`, pulled in by plotters' `evcxr_bitmap`).
  Otherwise the default implementation emits one `<rect>` per pixel.
- `present()` closes all open tags and, for `new`, writes the file.
  It runs once (`saved` flag).
  `Drop` calls it and ignores errors.
- `with_string`: the `String` is borrowed mutably by the backend,
  which lives in an `Rc<RefCell<_>>` shared by every `DrawingArea` and `ChartContext` derived from it.
  All of them must be dropped before the string can be read,
  and the document is only complete after `present()` or drop.

### `BitMapBackend` (feature `bitmap_backend`)

`BitMapBackend<'a, P: PixelFormat = RGBPixel>`.

| Constructor / method | Parameters | Requires | Notes |
|---|---|---|---|
| `BitMapBackend::new(path, (w, h))` | `path: &'a T`, `T: AsRef<Path> + ?Sized`; `(u32, u32)` | plotters `bitmap_encoder` (plotters-bitmap `image`), not wasm | RGB only. The buffer starts zeroed (black). Saved on `present()`. |
| `BitMapBackend::gif(path, (w, h), frame_delay)` | `path: T: AsRef<Path>`; `(u32, u32)`; `frame_delay: u32` (ms) | plotters `bitmap_gif` | Returns `Result<Self, BitMapBackendError>`. Each `present()` appends a frame. The delay is stored as `(ms + 5) / 10` centiseconds. The GIF loops forever. |
| `BitMapBackend::with_buffer(buf, (w, h))` | `buf: &'a mut [u8]` (RGB, 3 bytes/px) | none | Panics `"Wrong buffer size"` if `buf` is shorter than `w*h*3`. `present()` is a no-op. |
| `BitMapBackend::with_buffer_and_format::<P>(buf, (w, h))` | `buf: &'a mut [u8]` | none | Any `PixelFormat`. Returns `Result<Self, BitMapBackendError>` (`InvalidBuffer`). |
| `split(&mut self, area_size: &[u32])` | band heights in px | none | Returns `Vec<BitMapBackend<P>>` of horizontal bands, for parallel rendering. |

- Pixel formats (`plotters::backend::{RGBPixel, BGRXPixel, PixelFormat}`):
  `RGBPixel` is 3 bytes per pixel and can be saved.
  `BGRXPixel` is 4 bytes per pixel (framebuffers) and `can_be_saved()` is false.
  There is no alpha channel in the output.
  Alpha only blends onto what is already in the buffer.
- File output uses `image::ImageBuffer::save(path)`, so the format follows the file extension.
  plotters and plotters-bitmap enable only the `png`, `jpeg` and `bmp` features of `image` 0.24.
- There is no API for encoding to an in-memory PNG.
  In-memory output means rendering into `with_buffer` and encoding the RGB buffer with the `image` crate.
- `present()` saves (file), appends a frame (GIF) or does nothing (buffer).
  `ensure_prepared()`, called before every drawing operation, clears the `saved` flag,
  so drawing after `present()` makes `Drop` save again.
  `Drop` ignores errors.
- Errors are `BitMapBackendError::{InvalidBuffer, IOError, GifEncodingError, ImageError}`.
- Lines wider than 1 px are rasterized as filled polygons.
  1 px diagonal lines and circle edges are drawn with antialiasing.

### Text rendering engines

The engine is chosen at compile time (`src/style/font/mod.rs`):

| Engine | Selected when | Font source | Notes |
|---|---|---|---|
| `ttf` (`src/style/font/ttf.rs`) | feature `ttf` (default), non-wasm | font-kit `SystemSource` | Uses fontconfig on Linux. font-kit links `yeslogic-fontconfig-sys` and `freetype-sys` on Linux; `fontconfig-dlopen` loads fontconfig at run time instead. |
| `ab_glyph` (`src/style/font/ab_glyph.rs`) | feature `ab_glyph` **and not** `ttf`, non-wasm | fonts registered with `register_font` | Pure Rust, no system fonts. |
| naive (`src/style/font/naive.rs`) | neither feature, non-wasm | none | Layout estimate only (0.7 em per character). Drawing text on a bitmap panics with "The font implementation is unable to draw text" *(verified)*. |
| web (`src/style/font/web.rs`) | `wasm32` without WASI | the browser DOM | Measures with a `<span>`. |

**`ttf`** looks fonts up with `select_best_match(&[family, FamilyName::SansSerif], properties)`.
An unknown family therefore falls back to the system sans-serif, and `NoSuchFont` is returned only when nothing matches.
`FontStyle::Bold` maps to weight BOLD.
`Italic` and `Oblique` map to the style property.
Loaded fonts are cached per `"family"` or `"family, style"`.
There is no API for adding an in-memory font.
In 0.3.7 font loading reaches `unreachable!()` if font-kit returns a non-memory handle
(reported on macOS; fixed on master).

**`ab_glyph`** exposes `plotters::style::register_font`:

```rust
pub fn register_font(name: &str, style: FontStyle, bytes: &'static [u8]) -> Result<(), InvalidFont>
```

- Lookup is by the exact string `FontFamily::as_str()`: `"serif"`, `"sans-serif"`, `"monospace"` or the given name.
- A style that was not registered falls back to `FontStyle::Normal` of the same name *(verified)*.
  A name that was not registered gives `FontError::FontUnavailable` *(verified)*.
- `InvalidFont` lives in a private module and has no `Debug` impl, so `.unwrap()` does not compile.
  Use `.is_ok()` or `.map_err(...)`.
- If both `ttf` and `ab_glyph` are enabled, `ttf` renders text and registered fonts are ignored.
- plotters' own defaults all use the family `"sans-serif"`: mesh labels, legend text
  (`("sans-serif", 12)`) and `Pie` labels.
  Registering one font under that name covers them.
- `FontDesc` resolves its font data when it is constructed,
  so fonts must be registered before any `FontDesc` or `TextStyle` is built.

Minimal embedded-font setup, the shape used by the earlier duckers attempt:

```toml
plotters = { version = "0.3.7", default-features = false, features = [
    "bitmap_backend", "bitmap_encoder", "svg_backend", "ab_glyph",
    "line_series", "point_series", "area_series", "histogram",
] }
```

```rust
use plotters::prelude::*;
static DEJAVU: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
plotters::style::register_font("sans-serif", FontStyle::Normal, DEJAVU)
    .map_err(|_| "invalid font")?;
```

**Size semantics.** A `FontDesc` size is in px.
`ttf` rasterizes glyphs at an em size of `size / 1.24`.
`ab_glyph` scales glyphs with `size` itself.
SVG writes `size / 1.24`.
The same nominal size therefore gives slightly different text metrics depending on the engine.

**`FontDesc::style()` quirk.**
`FontDesc::style(s)` changes the reported style (SVG attributes) but keeps the glyph data loaded for the original style.
`("sans-serif", 20, FontStyle::Bold)` loads bold glyphs,
while `("sans-serif", 20).into_font().style(FontStyle::Bold)` renders the regular face in bitmaps.

### Size and DPI

There is no DPI or scale factor.
The backend size is in px, and every size parameter is px or relative to a drawing area.
SVG width and height are unitless, so a viewer can scale the output.
A high-resolution bitmap needs a larger backend size and correspondingly larger sizes, or relative sizes throughout.

### `present()`

`DrawingArea::present()` forwards to the backend's `present()`.
Both backends also present on drop and ignore errors there.
The crate docs recommend calling `present()` explicitly to see I/O errors.

## 2. Drawing areas

Source: `src/drawing/area.rs`.

`DrawingArea<DB, CT: CoordTranslate>` is a rectangle on a shared backend (`Rc<RefCell<DB>>`) with a coordinate spec.
`backend.into_drawing_area()` (trait `IntoDrawingArea`, implemented for every `DrawingBackend`) returns the root
`DrawingArea<DB, Shift>`, whose coordinates are px relative to the area's top left.
`DrawingArea` is `Clone` when `CT: Clone`.

### Methods on any coordinate spec

| Method | Parameters | Returns / notes |
|---|---|---|
| `fill(&color)` | `&C: Color` | Fills the whole area. |
| `draw(&element)` | an element whose points are in `CT::From` | Every point is mapped and then clamped to the area rectangle (`Rect::truncate`). |
| `draw_pixel(pos, &color)` | `pos: CT::From` | |
| `present()` | | Backend `present()`. |
| `dim_in_pixel()` | | `(u32, u32)` |
| `get_base_pixel()` | | Top-left corner in backend px. |
| `get_pixel_range()` | | `(Range<i32>, Range<i32>)` in backend px. |
| `relative_to_width(p)`, `relative_to_height(p)` | `p: f64`, clamped to 0..1 | `f64` px |
| `map_coordinate(&coord)` | `&CT::From` | `BackendCoord`, not clamped. |
| `estimate_text_size(text, &style)` | `&str`, `&TextStyle` | `(u32, u32)` |
| `strip_coord_spec()` | | `DrawingArea<DB, Shift>` with its origin at this area's top left. |
| `use_screen_coord()` | | `DrawingArea<DB, Shift>` with `Shift((0, 0))`: absolute backend coordinates, still clamped to this rectangle. |
| `into_coord_spec()`, `as_coord_spec()`, `as_coord_spec_mut()` | | The `CT`. |

### Methods only on `DrawingArea<DB, Shift>`

| Method | Parameters | Returns / notes |
|---|---|---|
| `margin(top, bottom, left, right)` | four `SizeDesc` (note the order) | A new inner area. |
| `shrink((left, top), (width, height))` | `SizeDesc` pairs; consumes `self` | Sub-rectangle at an offset. |
| `split_vertically(y)` | `SizeDesc` | `(upper, lower)` split at `y`. |
| `split_horizontally(x)` | `SizeDesc` | `(left, right)` split at `x`. |
| `split_evenly((rows, cols))` | `(usize, usize)` | `Vec<Self>` in row-major order. Remainder pixels go to the first cells. |
| `split_by_breakpoints(xs, ys)` | `xs: AsRef<[SizeDesc]>`, `ys: AsRef<[SizeDesc]>`, offsets from the area origin | `Vec<Self>` grid in row-major order. Breakpoints are sorted. |
| `titled(text, style)` | `&str`, `S: Into<TextStyle>` | Draws `text` centred at the top, with top padding `min(text_h / 2, 5)`, and returns the remaining area. |
| `draw_text(text, &style, pos)` | `&str`, `&TextStyle`, `pos: (i32, i32)` relative to the area | |
| `apply_coord_spec(coord_spec)` | any `CT: CoordTranslate` | `DrawingArea<DB, CT>` over the same rectangle. |

### Methods only on `DrawingArea<DB, Cartesian2d<X, Y>>`

`get_x_range()` and `get_y_range()` (data ranges), `get_x_axis_pixel_range()`, `get_y_axis_pixel_range()`,
and the low-level `draw_mesh(draw_func, y_count_max, x_count_max)`, which takes a closure.

## 3. ChartBuilder and ChartContext

Sources: `src/chart/builder.rs`, `src/chart/context.rs`, `src/chart/context/cartesian2d/mod.rs`,
`src/chart/context/cartesian3d/mod.rs`, `src/chart/dual_coord.rs`, `src/chart/state.rs`.

### ChartBuilder

`ChartBuilder::on(root: &'a DrawingArea<DB, Shift>)`.
The root must be a `Shift` area.
`SizeDesc` values here are resolved against the root area.

| Method | Parameters | Default |
|---|---|---|
| `margin(size)` | `SizeDesc` (negative becomes 0) | 0 on all sides |
| `margin_top(size)`, `margin_bottom(size)`, `margin_left(size)`, `margin_right(size)` | `SizeDesc` | 0 |
| `x_label_area_size(size)` | `SizeDesc`: bottom label area | 0 |
| `y_label_area_size(size)` | `SizeDesc`: left label area | 0 |
| `top_x_label_area_size(size)` | `SizeDesc`: top label area | 0 |
| `right_y_label_area_size(size)` | `SizeDesc`: right label area | 0 |
| `set_label_area_size(pos, size)` | `LabelAreaPosition::{Top, Bottom, Left, Right}`, `SizeDesc` | 0 |
| `set_left_and_bottom_label_area_size(size)` | `SizeDesc` | |
| `set_all_label_area_size(size)` | `SizeDesc` | |
| `caption(text, style)` | `S: AsRef<str>`, `Style: IntoTextStyle` | none |
| `build_cartesian_2d(x_spec, y_spec)` | two `AsRangedCoord` | returns `ChartContext<'c, DB, Cartesian2d<X::CoordDescType, Y::CoordDescType>>` |
| `build_cartesian_3d(x_spec, y_spec, z_spec)` | three `AsRangedCoord` | returns `ChartContext<..., Cartesian3d<...>>` |
| `build_ranged(x_spec, y_spec)` | | Deprecated alias of `build_cartesian_2d`. |

- All label area sizes default to 0.
  Axis lines, tick labels and axis descriptions are drawn **only inside label areas**,
  so a chart without label areas shows grid lines only.
- A **negative** label area size makes that label area overlap the plotting area instead of taking space from it.
  The mesh then defaults to inward ticks.
- `build_cartesian_2d` applies the margins and draws the caption with `titled`.
  It then splits the rest into a 3×3 grid.
  The centre cell is the plotting area, and the cells above, below, left and right of it are the label areas.
  The y pixel range is flipped, so larger y is higher on screen.
- `build_cartesian_3d` uses margins and the caption only.
  There are no label areas in 3D.
- `set_secondary_coord` is a method of `ChartContext`, not of `ChartBuilder`.

### ChartContext (2D and generic)

| Method | Parameters | Returns / notes |
|---|---|---|
| `configure_mesh()` | | `MeshStyle` (2D only), see [section 5](#5-mesh-and-axes-meshstyle) |
| `configure_series_labels()` | | `SeriesLabelStyle`, see [section 9](#9-legend-serieslabelstyle) |
| `draw_series(series)` | `S: IntoIterator<Item = R>`, `R: Borrow<E>`, `E: Drawable` with points in the chart's coordinates | `Result<&mut SeriesAnno>` for `.label()` and `.legend()` |
| `plotting_area()` | | `&DrawingArea<DB, CT>`, for drawing arbitrary elements in data coordinates |
| `as_coord_spec()` | | `&CT` |
| `x_range()`, `y_range()` | | `Range<X::ValueType>`, `Range<Y::ValueType>` (2D) |
| `backend_coord(&(x, y))` | | `BackendCoord`, not clamped (2D) |
| `set_secondary_coord(self, x_spec, y_spec)` | two `AsRangedCoord`; consumes `self` | `DualCoordChartContext` (2D) |
| `into_coord_trans(self)` | requires `CT: ReverseCoordTranslate` | `impl Fn(BackendCoord) -> Option<CT::From>`, pixel to data |
| `into_chart_state(self)`, `to_chart_state(&self)`, `into_shared_chart_state(self)` | | `ChartState<CT>` (or `ChartState<Arc<CT>>`) |

`ChartState::restore(&area)` rebuilds a `ChartContext` over the same plotting rectangle, without label areas.

### ChartContext (3D only)

| Method | Parameters | Notes |
|---|---|---|
| `configure_axes()` | | `Axes3dStyle`, see [section 10](#10-3d) |
| `with_projection(pf)` | `pf: FnOnce(ProjectionMatrixBuilder) -> ProjectionMatrix` | Returns `&mut Self`. |
| `set_3d_pixel_range((x, y, z))` | `(i32, i32, i32)` px | Sets the size of the 3D box. **Resets the projection** to the default builder. |

### DualCoordChartContext

Returned by `set_secondary_coord`.
It `Deref`s to the primary `ChartContext`.

| Method | Notes |
|---|---|
| `configure_secondary_axes()` | `SecondaryMeshStyle` |
| `draw_secondary_series(series)` | Draws in the secondary coordinates. The `SeriesAnno` is stored with the primary, so it appears in the same legend. Only 2D elements. |
| `secondary_plotting_area()`, `borrow_secondary()` | |
| `into_secondary_coord_trans()`, `into_coord_trans_pair()` | Pixel-to-data closures. |
| `into_chart_state()`, `to_chart_state()`, `into_shared_chart_state()` | `DualCoordChartState`, with `restore(&area)`. |

After `set_secondary_coord` the primary keeps the bottom and left label areas,
and the secondary gets the top x and right y label areas.

## 4. Coordinate specs and ranges

Sources: `src/coord/ranged1d/mod.rs`, `src/coord/ranged1d/discrete.rs`,
`src/coord/ranged1d/types/{numeric,slice,datetime}.rs`, `src/coord/ranged1d/combinators/*.rs`,
`src/coord/ranged2d/cartesian.rs`, `src/data/{float,data_range}.rs`.

### The traits

- `Ranged`: `type ValueType`, `type FormatOption` (`DefaultFormatting` or `NoDefaultFormatting`),
  `map(&v, (px0, px1)) -> i32`, `key_points(hint) -> Vec<ValueType>`, `range() -> Range<ValueType>`,
  `axis_pixel_range(limit)`.
- `AsRangedCoord`: anything accepted by `build_cartesian_2d/3d`.
  Every `Ranged` type is itself `AsRangedCoord`.
- `DiscreteRanged: Ranged`: `size()`, `index_of(&v)`, `from_index(i)`, `values()`, `previous(&v)`, `next(&v)`.
  Required by `Histogram`, `into_segmented`, `group_by` and `nested_coord`.
- `ReversibleRanged`: `unmap(px, limit) -> Option<ValueType>`.
  Implemented for floats, every `DiscreteRanged` and `RangedDateTime`.
- `KeyPointHint`: `usize`, `BoldPoints(n)` (labels and bold grid), `LightPoints::new(bold_count, max)` (light grid).
- `ValueFormatter<V>`: `format(&V) -> String`, `format_ext(&self, &V)`.
  `FormatOption = DefaultFormatting` gives a blanket implementation that formats with `{:?}`.

### Value types that can be passed as a range

| Expression | Coordinate type | Value type | Discrete | Default label format | Feature |
|---|---|---|---|---|---|
| `a..b` with `f32` / `f64` | `RangedCoordf32` / `RangedCoordf64` | `f32` / `f64` | no | `FloatPrettyPrinter { allow_scientific: false, min_decimal: 1, max_decimal: 5 }`, e.g. `0.0`, `2.5`, `10.0` *(verified)* | none |
| `a..b` with `i32`, `u32`, `i64`, `u64`, `i128`, `u128`, `isize`, `usize` | `RangedCoordi32`, ... | same | yes, `size = b - a + 1` | `{:?}` | none |
| `&slice[..]` (`&'a [T]`, `T: PartialEq`) | `RangedSlice<'a, T>` (`plotters::coord::types::RangedSlice`) | `&'a T` | yes | `{:?}`, so strings get quotes: `"a"` *(verified)* | none |
| `a..b` with `NaiveDate` or `Date<Tz>` | `RangedDate<D>` | `D` | yes (days) | `{:?}` | `chrono` |
| `a..b` with `DateTime<Tz>` | `RangedDateTime<DateTime<Tz>>` | `DateTime<Tz>` | no | `{:?}` | `chrono` |
| `RangedDateTime::from(a..b)` with `NaiveDateTime` | `RangedDateTime<NaiveDateTime>` | `NaiveDateTime` | no | `{:?}` | `chrono` |
| `a..b` with `chrono::Duration` | `RangedDuration` | `Duration` | no | `{:?}` | `chrono` |
| `(a..b).monthly()` / `(a..b).yearly()` (traits `IntoMonthly`, `IntoYearly`) | `Monthly<T>` / `Yearly<T>` | `T` | yes | `"{year}-{month}"` for both, e.g. `2024-3` | `chrono` |

- Only `std::ops::Range` (`a..b`) is accepted.
  There is no `RangeInclusive` support.
  `b` is included in the axis: it maps to the far edge.
- There are no coordinate types for `i8`, `i16`, `u8`, `u16`, and none for strings other than slices.
- `Range<NaiveDateTime>` itself is not `AsRangedCoord` in 0.3.7, so it must be wrapped in `RangedDateTime::from(...)`.
  `Monthly`/`Yearly` require `Range<T>: AsRangedCoord`, which rules out `NaiveDateTime`.
- Integer key points step by 1, 2 or 5 × 10ⁿ.
  Float key points use the same steps.
- `RangedDate` key points are every day, else every week, else every `ceil(weeks / max)` weeks,
  starting at the range start.
- `RangedDateTime` key points use periods of 1/2/5 × 10ⁿ ns, then 1, 2, 5, 10, 15, 20, 30 seconds or minutes,
  then 1, 2, 4, 8, 12 hours, then dates.
- `Monthly` key points are monthly, then quarterly, then half-yearly, then yearly.
- A `RangedSlice` value that is not in the slice maps to the start of the axis.
- There is no dedicated reverse combinator.
  A range with `start > end` maps in reverse, e.g. `10.0..0.0` puts 0 at the top of a y axis *(verified)*.

### Combinators

| Method (trait) | Applies to | Result | Parameters and defaults |
|---|---|---|---|
| `.log_scale()` (`IntoLogRange`) | `Range<T: LogScalable>`: `u8`..`usize`, `i8`..`isize`, `f32`, `f64` | `LogRangeExt<T>`, which becomes `LogCoord<T>` | `.base(f64)`, default 10. `.zero_point(T)`, default 0. |
| `.into_segmented()` (`IntoSegmentedCoord`) | any discrete range | `SegmentedCoord<D>`, value `SegmentValue<T>` | none |
| `.step(s)` (`IntoLinspace`) | any range whose value type is `Add<S>` | `Linspace<_, S, Exact>` (discrete) | `.use_round()`, `.use_floor()`, `.use_ceil()`, `.use_exact()` (default exact) choose how `index_of` matches values to grid points. |
| `.group_by(n)` (`ToGroupByRange`) | any discrete range | `GroupBy<D>` | Groups `n` consecutive values per bucket and key point. |
| `.partial_axis(sub_range)` (`IntoPartialAxis`) | any range | `PartialAxis<R>` | The axis line covers only `sub_range`. `make_partial_axis(axis_range, part: Range<f64>)` builds a full range from a fraction. |
| `.nested_coord(\|cat\| sub_range)` (`BuildNestedCoord`) | any discrete range | `NestedRange<P, S>`, value `NestedValue<C, V>` | The closure builds one secondary range per primary value. |
| `.with_key_points(vec)` (`BindKeyPoints`) | any range | `WithKeyPoints<R>` | `Vec<V>` of bold key points. `.with_light_points(iter)`; the light list defaults to empty. |
| `.with_key_point_func(f)` (`BindKeyPointMethod`) | any range | `WithKeyPointMethod<R>` | `f: Fn(usize) -> Vec<V> + 'static`. `.with_light_point_func(f)`. |

- **Log scale.**
  Mapping is linear in `ln(v - zero_point)`, and the base only affects key points.
  A range bound equal to the zero point is replaced by `other_bound * 1e-5`.
  Integer 0 is treated as 0.5.
  If either bound is below the zero point, the whole range is mirrored (negative log axis).
  A range spanning the zero point is not supported.
  Labels use `{:?}`, e.g. `1.0`, `10.0`, ..., `1000000.0` *(verified)*.
  `LogRangeExt::base()` checks the current base (always 10) instead of the argument, so any value is accepted.
  `LogRange(Range)` is a deprecated wrapper for the same thing.
- **Segmented.**
  `SegmentValue::{Exact(T), CenterOf(T), Last}`, with `From<T>` giving `Exact`.
  Each discrete value gets a band.
  `Exact(v)` is the band's left edge and `CenterOf(v)` its centre.
  Key points and labels are `CenterOf`, so labels are centred in the bands.
  `Last` is the right end and is formatted as `""`.
  A float axis becomes segmentable through `.step(s).use_round().into_segmented()`.
- **Nested.**
  `NestedValue::{Category(C), Value(C, V)}` with `From<(C, V)>` and `From<C>`.
  Every category gets an equal-width slot, and the secondary range maps inside it.
  The label for `Category` uses the primary formatter, the label for `Value` the secondary one.
- **Custom key points.**
  In 0.3.7 `WithKeyPoints` over `f32`/`f64` has no `ValueFormatter`,
  so `configure_mesh()` does not compile for such an axis *(verified)*.
  Integer and log axes work (`examples/tick_control.rs`).
  Fixed on master.

### Label formatting defaults

- Floats: `FloatPrettyPrinter` as above.
  `plotters::data::float::pretty_print_float(n, allow_sn)`
  and `FloatPrettyPrinter { allow_scientific, min_decimal, max_decimal }` are public.
- Everything with `DefaultFormatting` uses `{:?}`: integers, slices, dates and times, durations, `LogCoord`,
  `PartialAxis`.
- `Monthly`/`Yearly`: `"{year}-{month}"`, month not zero-padded.
- `Linspace`, `GroupBy` and `WithKeyPoints` delegate to the inner coordinate.
  `SegmentedCoord` and `NestedRange` format as described above.
- Formatters are replaced per axis with closures (`x_label_formatter` and friends).

### Mapping details

- Numeric `map` is `px0 + (px1 - px0) * (v - start) / (end - start)`, truncated towards `px0` with a 1e-3 tolerance.
  It is linear outside the range as well.
- A degenerate range (`start == end`) maps every value to `(px1 - px0) / 2`, which is not offset by `px0`.
- NaN maps to `px0`, the start of the axis' pixel range *(verified)*.
  Key-point generation asserts that float range bounds are not NaN.
- `Cartesian2d` maps x and y independently.
  Its pixel ranges are set by `build_cartesian_2d`.

### Range helper

`plotters::data::fitting_range(iter: impl IntoIterator<Item = &T>) -> Range<T>` returns `min..max`,
or `0..1` for an empty iterator, with no padding.
`T: Zero + One + PartialOrd + Clone`.

## 5. Mesh and axes (`MeshStyle`)

Sources: `src/chart/mesh.rs`, `src/chart/context/cartesian2d/draw_impl.rs`.

`chart.configure_mesh()` returns `MeshStyle<'a, 'b, X, Y, DB>`.
Its `SizeDesc` parameters are resolved against the plotting area.

| Method | Parameters | Default |
|---|---|---|
| `x_desc(text)`, `y_desc(text)` | `Into<String>` | none |
| `axis_desc_style(style)` | `IntoTextStyle` | the x label style |
| `x_labels(n)`, `y_labels(n)` | `usize`: maximum number of labels and bold lines | 11 |
| `x_label_formatter(f)`, `y_label_formatter(f)` | `&'b dyn Fn(&X::ValueType) -> String` | the coordinate's `ValueFormatter` |
| `label_style(style)` | `IntoTextStyle`, sets both axes | `("sans-serif", min(12, 12 % of the smaller plotting-area side))`, black |
| `x_label_style(style)`, `y_label_style(style)` | `IntoTextStyle` | as above |
| `x_label_offset(v)`, `y_label_offset(v)` | `SizeDesc`: shifts labels along the axis (x labels horizontally, y labels vertically) | 0 |
| `x_max_light_lines(n)`, `y_max_light_lines(n)`, `max_light_lines(n)` | `usize`: light lines per bold interval; the light key-point limit is `labels × n` | 10 |
| `light_line_style(style)` | `Into<ShapeStyle>` | `BLACK.mix(0.1)`, 1 px |
| `bold_line_style(style)` | `Into<ShapeStyle>` | `BLACK.mix(0.2)`, 1 px |
| `axis_style(style)` | `Into<ShapeStyle>`: axis line and tick marks | `BLACK`, 1 px |
| `disable_x_mesh()`, `disable_y_mesh()`, `disable_mesh()` | | grid drawn. `disable_x_mesh` removes the vertical lines at x key points, light and bold. |
| `disable_x_axis()`, `disable_y_axis()`, `disable_axes()` | | axes drawn. Disabling removes the axis line, tick labels and ticks; the axis description is still drawn. |
| `set_tick_mark_size(pos, v)` | `LabelAreaPosition`, `SizeDesc` | `min(5, 5 % of the smaller plotting-area side)` px, negated where the label area overlaps the plotting area |
| `set_all_tick_mark_size(v)` | `SizeDesc` | as above |
| `draw()` | | Draws. Returns `Result<(), DrawingAreaErrorKind>`. |

- `draw()` makes two passes: light lines plus axis descriptions, then bold lines plus axes, ticks and labels.
- Labels are placed in every label area that exists, so a top label area repeats the x labels.
- A label whose position falls outside the axis pixel range is skipped.
- Right-hand y labels are right-aligned to a common width (at most twice the narrowest label).
  Labels at least that wide are left-aligned instead.
- `y_desc` is rotated 270° on the left and 90° on the right.
- **Negative tick size (0.3.7):** the ticks point into the plotting area,
  the labels move to the inner edge of the label area, and the axis line is drawn on the inner edge too.
  On master a negative size only flips the tick direction.

`SecondaryMeshStyle` (from `configure_secondary_axes()`) always has mesh lines disabled.
It offers `axis_style`, `x_label_offset`, `y_label_offset`, `x_labels`, `y_labels`, `x_label_formatter`,
`y_label_formatter`, `axis_desc_style`, `x_desc`, `y_desc`, `label_style`, `set_all_tick_mark_size`,
`set_tick_mark_size` and `draw`.
There are no per-axis label styles and no `disable_*` methods.

## 6. Series

Sources: `src/series/{line_series,area_series,histogram,point_series,surface}.rs`, `src/chart/series.rs`.

| Series | Constructor | Builder methods (default) | Yields | Feature |
|---|---|---|---|---|
| `LineSeries<DB, Coord>` | `new(iter: IntoIterator<Item = Coord>, style: Into<ShapeStyle>)` | `point_size(u32)` px (0 = no markers) | `DynElement<'static, DB, Coord>`: one `Circle` per point if `point_size > 0`, then one `PathElement` | `line_series` |
| `DashedLineSeries<I, Size>` | `new(points: IntoIterator (Clone iterator), size: Size, spacing: Size, style: ShapeStyle)` | none | one `DashedPathElement`. `size` is the dash length and `spacing` the gap, both `SizeDesc` px. | `line_series` |
| `DottedLineSeries<I, Size, Marker>` | `new(points, shift: Size, spacing: Size, func: Fn(BackendCoord) -> Marker + 'static)` | none | one `DottedPathElement`: a marker every `spacing` px along the path, the first at `shift` | `line_series` |
| `AreaSeries<DB, X, Y>` | `new(iter: IntoIterator<Item = (X, Y)>, baseline: Y, area_style: Into<ShapeStyle>)` | `border_style(Into<ShapeStyle>)` (`TRANSPARENT`) | a `Polygon` (always filled), then a `PathElement` border along the data | `area_series` |
| `Histogram<'a, BR, A, Vertical \| Horizontal>` | `Histogram::vertical(&chart)` (x axis discrete) or `Histogram::horizontal(&chart)` (y axis discrete) | `style(Into<ShapeStyle>)` (`GREEN.filled()`), `style_func(Fn(&BR::ValueType, &A) -> ShapeStyle)`, `baseline(A)` (`A::default()`), `baseline_func(Fn(&BR::ValueType) -> A)`, `margin(u32)` px on each side of a bar (5), `data(IntoIterator<Item = (TB: Into<BR::ValueType>, A)>)` | one `Rectangle` per non-empty bucket | `histogram` |
| `PointSeries<'a, Coord, I, E, Size>` | `new(iter, size: Size, style: Into<ShapeStyle>)` with `E: PointElement` chosen by type (`Circle`, `Cross`, `TriangleMarker`, `Pixel`) | none | one `E` per item | `point_series` |
| | `of_element(iter, size, style, cons: &'a F)` with `F: Fn(Coord, Size, ShapeStyle) -> E` | none | one `E` per item, any element | `point_series` |
| `SurfaceSeries<'a, X, Y, Z, D, F>` | `xoy(x_iter, y_iter, f: Fn(X, Y) -> Z)`, `xoz(x_iter, z_iter, f: Fn(X, Z) -> Y)`, `yoz(y_iter, z_iter, f: Fn(Y, Z) -> X)`, `new(...)` | `style(Into<ShapeStyle>)` (`BLUE.mix(0.4).filled()`), `style_func(&'a F: Fn(&Output) -> ShapeStyle)` | one quad `Polygon<(X, Y, Z)>` per grid cell | `surface_series` |

- `LineSeries` and `AreaSeries` produce `DynElement<'static, ...>` and require `Coord: Clone + 'static`.
  A category axis over run-time data (`RangedSlice` of non-`'static` strings) cannot use them.
  `PathElement` or `Polygon` passed to `draw_series` work *(verified)*.
- `LineSeries` markers are `Circle`s filled according to `style.filled`.
- `DashedLineSeries::new` takes a `ShapeStyle` by value, not `Into<ShapeStyle>`.
- The `Histogram` bucket axis type `BR` must be `DiscreteRanged + Clone`,
  and the value type `A` must be `AddAssign + Default` and equal to the other axis' value type.
  `vertical`/`horizontal` read the discrete axis from the chart passed in.
- `PointSeries::new` usually needs the element type spelled out, e.g.
  `PointSeries::<_, _, Circle<_, _>, _>::new(data, 5, &RED)`.
- `SurfaceSeries` takes `Iterator`s (not `IntoIterator`) of sample positions and a function.
  There is no constructor that takes a grid of precomputed values.

**Series annotations.**
`draw_series` returns `&mut SeriesAnno<'a, DB>`:

| Method | Parameters | Notes |
|---|---|---|
| `label(text)` | `Into<String>` | Legend text. |
| `legend(func)` | `Fn(BackendCoord) -> E + 'a`, `E: IntoDynElement<'a, DB, BackendCoord>` | Builds the legend glyph in px, see [section 9](#9-legend-serieslabelstyle). |

A series without a label and without a legend function is left out of the legend.

`ErrorBar`, `Boxplot` and `CandleStick` are not series.
They are drawn with `draw_series(rows.map(|r| ErrorBar::new_vertical(...)))`.

## 7. Elements

Sources: `src/element/*.rs`, `src/data/quartiles.rs`.
All elements implement `Drawable` plus `PointCollection` in some coordinate type.
Only the points returned by `PointCollection` are mapped from data to px, and they are then clamped to the area.
All other sizes are px.
Relative (`SizeDesc`) sizes are resolved against the area being drawn on, which is the plotting area for chart series.

| Element | Constructor | Data-space parameters | Pixel-space parameters | Style behaviour | Feature |
|---|---|---|---|---|---|
| `Pixel<Coord>` | `new(pos: Into<Coord>, style)` | `pos` | none | colour only | none |
| `Circle<Coord, Size>` | `new(center, size: SizeDesc, style)` | `center` | radius `size` | `filled` respected, `stroke_width` for the outline | none |
| `Cross<Coord, Size>` | `new(center, size, style)` | `center` | half-size | two diagonal lines, `stroke_width` | none |
| `TriangleMarker<Coord, Size>` | `new(center, size, style)` | `center` | circumradius | always filled | none |
| `Rectangle<Coord>` | `new(points: [Coord; 2], style)`; `set_margin(t, b, l, r)`, `set_style(s)`, `get_points()` | the two corners | margin insets (u32) | `filled` respected. No separate outline colour. | none |
| `Polygon<Coord>` | `new(points: Into<Vec<Coord>>, style)` | vertices | none | always filled, colour only, no outline | none |
| `PathElement<Coord>` | `new(points: Into<Vec<Coord>>, style)` | vertices | none | open polyline, `stroke_width` | none |
| `DashedPathElement<I, Size>` | `new(points, size, spacing, style)` | vertices | dash and gap | as `PathElement` | none |
| `DottedPathElement<I, Size, Marker>` | `new(points, shift, spacing, func: Fn(BackendCoord) -> Marker + 'static)` | vertices | shift, spacing; the marker is built at a px position | marker's own | none |
| `Text<'a, Coord, T: Borrow<str>>` | `new(text, pos, style: Into<TextStyle>)` | anchor `pos` | font size | anchor from `TextStyle::pos` | none |
| `MultiLineText<'a, Coord, T>` | `new(pos, style)`, `push_line(l)`, `set_line_height(f64)` (1.25), `from_str(text, pos, style, max_width: u32)`, `from_string(...)`, `estimate_dimension()`, `relocate(pos)`, `compute_line_layout()` | anchor `pos` | line height (× font size), wrap width (0 = no wrapping) | left-aligned only | none |
| `Pie<'a, (i32, i32), Label: Display>` | `new(center: &(i32, i32), radius: &f64, sizes: &[f64], colors: &[RGBColor], labels: &[Label])`; `start_angle(deg)` (0 = 3 o'clock, clockwise), `label_style(Into<TextStyle>)` (sans-serif, 5 % of radius, black), `label_offset(f64 px)` (5 % of radius), `percentages(Into<TextStyle>)` (off; prints `{:.1}%`), `donut_hole(radius)` (0) | none | everything | wedges filled with `RGBColor` (no alpha) | none |
| `Boxplot<K, O>` | `new_vertical(key: K, &Quartiles)`, `new_horizontal(key, &Quartiles)`; `style(Into<ShapeStyle>)` (`BLACK`), `width(u32)` (10), `whisker_width(f64)` (1.0 × width), `offset(Into<f64>)` (0) | `key`, and the five quartile values on an **`f32`** value axis | box width, whisker width, offset along the key axis | box never filled | `boxplot` |
| `ErrorBar<K, V, O>` | `new_vertical(key, min, avg, max, style, width: u32)`, `new_horizontal(...)` | key, min, avg, max | cap width; circle radius `width / 2` at `avg` | circle `filled` respected | `errorbar` |
| `CandleStick<X, Y>` | `new(x, open, high, low, close, gain_style, loss_style, width: u32)` | x, open, high, low, close | body width | gain style if `open < close`, else loss style; body `filled` respected | `candlestick` |
| `BitMapElement<'a, Coord, P>` | `new(pos, (w, h))`, `with_owned_buffer(pos, size, Vec<u8>)`, `with_mut(pos, size, &mut [u8])`, `with_ref(pos, size, &[u8])`, `copy_to(pos)`, `move_to(pos)`, `as_bitmap_backend()`; `From<(Coord, image::DynamicImage)>` (feature `image`) | top-left `pos` | image size | RGB blit | `bitmap_backend` |
| `EmptyElement<Coord, DB>` | `EmptyElement::at(coord)` then `+ element + element ...` (building `BoxedElement` and `ComposedElement`) | the anchor | the added elements' coordinates are px offsets from the anchor | per element | none |
| `DynElement<'a, DB, Coord>` | `element.into_dyn()` (`IntoDynElement`) | as wrapped | as wrapped | type erasure, used by `LineSeries`, `AreaSeries` and legends | none |
| `Cubiod<X, Y, Z>` (sic) | `new([(x0, y0, z0), (x1, y1, z1)], face_style, edge_style)` | both corners | none | faces filled, edges stroked, faces sorted by depth | none |

- **`Pie`** is drawable only on an area whose coordinate type is `(i32, i32)`, such as a root `Shift` area.
  It ignores the mapped position and draws at `center` in absolute backend px, so a sub-area's offset is not applied.
  Mismatched slice lengths return an error (reported as a `FontError`).
- **`Boxplot`** whiskers end at the fences `Q1 − 1.5·IQR` and `Q3 + 1.5·IQR`, which are not clamped to the data,
  and no outliers are drawn.
- **`Quartiles::new(&[T: Into<f64> + Copy + PartialOrd])`** sorts a copy and interpolates percentiles linearly.
  It panics on an empty slice and on NaN.
  Other methods: `values() -> [f32; 5]` (lower fence, Q1, median, Q3, upper fence) and `median() -> f64`.
  Fields are private, so precomputed quartiles cannot be passed in.
- **`CandleStick`** draws a line from open to high, a line from low to close, and the body between open and close.
- The `Path` alias of `PathElement` is deprecated and exported only with `deprecated_items`.
- `DashedPathElement`, `DottedPathElement`, `ComposedElement`, `BoxedElement`, `ErrorBarOrientH/V`, `BoxplotOrientH/V`
  and the `PointElement` trait are in `plotters::element`, not in the prelude.

## 8. Styles

Sources: `src/style/{color,palette,shape,size,text}.rs`, `src/style/colors/{mod,full_palette,colormaps}.rs`,
`src/style/font/font_desc.rs`, `plotters-backend-0.3.7/src/text.rs`.

### Colours

| Item | Definition | Notes |
|---|---|---|
| `trait Color` | `to_backend_color()`, `rgb() -> (u8, u8, u8)`, `alpha() -> f64`, `mix(f64) -> RGBAColor` (multiplies alpha), `to_rgba()`, `filled() -> ShapeStyle`, `stroke_width(u32) -> ShapeStyle` | Implemented for the types below and for `&T`. |
| `RGBColor` | `RGBColor(pub u8, pub u8, pub u8)` | |
| `RGBAColor` | `RGBAColor(pub u8, pub u8, pub u8, pub f64)`, alpha in 0..1 | |
| `HSLColor` | `HSLColor(pub f64, pub f64, pub f64)`: hue, saturation, lightness, **all in 0..1** | 0.3.7 clamps each component to 0..1, so the hue is a fraction of a turn and does not wrap. |
| Named colours | `WHITE`, `BLACK`, `RED` (255, 0, 0), `GREEN` (0, 255, 0), `BLUE`, `YELLOW`, `CYAN`, `MAGENTA` (`RGBColor`); `TRANSPARENT` = `RGBAColor(0, 0, 0, 0.0)` | In the prelude. |
| `full_palette` | 287 `RGBColor` constants: `WHITE`, `BLACK` and 19 Material Design families (`RED`, `PINK`, `PURPLE`, `DEEPPURPLE`, `INDIGO`, `BLUE`, `LIGHTBLUE`, `CYAN`, `TEAL`, `GREEN`, `LIGHTGREEN`, `LIME`, `YELLOW`, `AMBER`, `ORANGE`, `DEEPORANGE`, `BROWN`, `GREY`, `BLUEGREY`), each as base, `_50`, `_100` to `_900`, `_A100`, `_A200`, `_A400`, `_A700` | Feature `full_palette`. The base name equals the `_500` tint, so `full_palette::RED` is (244, 67, 54), unlike `style::RED`. The prelude exports the module, not the constants. |
| `trait Palette` | `const COLORS: &[(u8, u8, u8)]`, `pick(idx) -> PaletteColor<Self>` (index modulo length) | |
| `Palette99`, `Palette9999`, `Palette100` | 21, 9 and 4 colours | "accessibility" palettes |
| `PaletteColor<P>` | `PaletteColor::<P>::pick(idx)` | alpha 1 |
| `trait ColorMap<C, F = f32>` | `get_color(h)` for h in 0..1, `get_color_normalized(h, min, max)` | Feature `colormaps`. Values are clamped to the bounds and colours interpolated linearly between evenly spaced stops. |
| Predefined colour maps | `ViridisRGBA`, `ViridisRGB`, `BlackWhite`, `MandelbrotHSL`, `VulcanoHSL`, `Bone`, `Copper`, each also with associated `get_color(h)` / `get_color_normalized(h, min, max)` | Feature `colormaps`. |
| `DerivedColorMap<C>` | `DerivedColorMap::new(&[C])` for `RGBColor`, `RGBAColor`, `HSLColor` | Feature `colormaps`. Also the `def_linear_colormap!` macro. |

### `ShapeStyle`

`ShapeStyle { pub color: RGBAColor, pub filled: bool, pub stroke_width: u32 }`.
`From<T: Color>` gives `filled: false, stroke_width: 1`.
Methods `filled()` and `stroke_width(u32)`.
There are no dash patterns (use `DashedLineSeries`/`DashedPathElement`), no line caps or joins,
and no separate fill and stroke colours.
A stroke width of 0 draws nothing in the rasterizer.

### Fonts and text styles

| Item | Definition |
|---|---|
| `FontFamily<'a>` | `Serif`, `SansSerif`, `Monospace`, `Name(&'a str)`. `From<&str>` maps `serif`, `sans-serif`, `monospace` (case-insensitive) to the generic families. `as_str()`. |
| `FontStyle` | `Normal`, `Oblique`, `Italic`, `Bold`; no bold italic. `From<&str>`: `normal`, `italic`, `oblique`, `bold`, anything else gives `Normal`. |
| `FontTransform` | `None`, `Rotate90`, `Rotate180`, `Rotate270` (clockwise). No other angles. |
| `FontDesc<'a>` | `new(family, size: f64, style)`, `resize(size)`, `style(style)` (see the quirk in section 1), `transform(t)`, `color(&C) -> TextStyle`, `get_family()`, `get_name()`, `get_style()`, `get_size()`, `get_transform()`, `layout_box(text)`, `box_size(text)`, `draw(...)` |
| `From<...> for FontDesc` | `&str` and `FontFamily` (size 12, `Normal`); `(family, size: Into<f64>)`; `(family, size, style: Into<FontStyle>)`, where `family` is `&str` or `FontFamily` |
| `IntoFont` | `.into_font()` for every `T: Into<FontDesc>` |
| `TextStyle<'a>` | `pub font: FontDesc`, `pub color: BackendColor`, `pub pos: text_anchor::Pos`. Methods `color(&'a C)`, `transform(FontTransform)`, `pos(Pos)`. `From<T: Into<FontDesc>>` gives black and top-left. |
| `IntoTextStyle<'a>` | `into_text_style(&parent)` for `FontDesc`, `TextStyle`, `&str`, `FontFamily`, `u32` / `f64` (sans-serif of that size), `&C: Color` (sans-serif 12 in that colour), `(family, SizeDesc)`, `(family, SizeDesc, &C)`, `(family, SizeDesc, FontStyle)`, `(family, SizeDesc, FontStyle, &C)`. Builders `.with_color(c)` and `.with_anchor::<C>(pos)`. |
| `text_anchor::Pos` | `Pos::new(HPos, VPos)`, `HPos::{Left (default), Right, Center}`, `VPos::{Top (default), Center, Bottom}`. The anchor is relative to the text regardless of rotation. |

- `IntoTextStyle` resolves relative sizes against a parent:
  the root area for `caption`, the plotting area for mesh and legend styles.
- `titled`, `Text::new`, `Pie::label_style` and `Axes3dStyle::label_style` take `Into<TextStyle>` instead,
  which excludes the colour tuples and relative sizes.

### Relative sizes

`AsRelative` is implemented for every `T: Into<f64>`: `percent_width()`, `percent_height()`, and `percent()`
(of the smaller side) return `RelativeSize::{Width, Height, Smaller}`.
`.min(px)` sets a lower bound and `.max(px)` an upper bound (`RelativeSizeWithBound`).
`.max(n)` caps the result at `n`, so `12.percent().max(12)` means "12 % but at most 12 px".

## 9. Legend (`SeriesLabelStyle`)

Source: `src/chart/series.rs`.

`chart.configure_series_labels()` returns `SeriesLabelStyle`.
The legend is drawn inside the plotting area, in px relative to its top left.

| Method | Parameters | Default |
|---|---|---|
| `position(pos)` | `SeriesLabelPosition` | `MiddleRight` |
| `margin(v)` | `SizeDesc`, padding inside the box | 10 |
| `legend_area_size(v)` | `SizeDesc`, width of the glyph column | 30 |
| `border_style(s)` | `Into<ShapeStyle>` | `TRANSPARENT` |
| `background_style(s)` | `Into<ShapeStyle>`, always drawn filled | `TRANSPARENT` |
| `label_font(f)` | `IntoTextStyle` | `("sans-serif", 12)`, black |
| `draw()` | | |

- `SeriesLabelPosition`: `UpperLeft`, `MiddleLeft`, `LowerLeft`, `UpperMiddle`, `MiddleMiddle`, `LowerMiddle`,
  `UpperRight`, `MiddleRight`, `LowerRight` (5 px from the plotting-area edges, or centred),
  and `Coordinate(i32, i32)` (top left of the box, px from the plotting-area origin).
- The box is `max label width + legend_area_size + 2 × margin` wide.
  Labels are left-aligned in a `MultiLineText` with a line height of 1.25 × the font size.
- Glyphs come from the `legend` closure on each `SeriesAnno`.
  It is called once per entry with `(box_x + margin, vertical centre of the entry's text line)`,
  the **left** edge of the glyph column *(verified: `(15, 20)` for `UpperLeft` with the defaults)*.
  The glyph is expected to fit in `x .. x + legend_area_size`.
  The doc comment says the point is "mid-right", which does not match the code.
- An entry with a label but no legend function gets an empty glyph.
- `draw()` draws over whatever is already drawn, so it is called after the series.

## 10. 3D

Sources: `src/chart/builder.rs`, `src/chart/context/cartesian3d/*.rs`, `src/chart/axes3d.rs`,
`src/coord/ranged3d/{cartesian3d,projection}.rs`, `src/element/basic_shapes_3d.rs`, `src/series/surface.rs`.

- `ChartBuilder::build_cartesian_3d(x, y, z)` gives `ChartContext<DB, Cartesian3d<X, Y, Z>>` with data points
  `(x, y, z)`.
  Any 2D element type works with 3D coordinates, e.g. `LineSeries` over `(x, y, z)`.
- Each axis is mapped to the pixel range from 0 to `coord_size`.
  The default `coord_size` is 4/5 of the smaller plotting-area side for all three.
  `set_3d_pixel_range((x, y, z))` changes it and resets the projection.
- `Cartesian3d::translate` multiplies by a `ProjectionMatrix`.
  The default is `ProjectionMatrix::rotate(PI, 0, 0)`, which flips the screen y so that larger Y is higher on screen.
  The examples plot height fields with `SurfaceSeries::xoz` (y = f(x, z)).
- `with_projection(|pb| ...)` receives a `ProjectionMatrixBuilder` whose pivot is already set
  (centre of the 3D box to centre of the plotting area).
  It must return a `ProjectionMatrix`, usually `pb.into_matrix()`.

`ProjectionMatrixBuilder`:

| Field / method | Type | Default |
|---|---|---|
| `yaw` | `pub f64` (radians) | 0.5 |
| `pitch` | `pub f64` (radians) | 0.15 |
| `scale` | `pub f64` | 1.0 |
| `set_pivot(before, after)` | `(i32, i32, i32)`, `(i32, i32)` | set by `with_projection` |
| `new()`, `into_matrix()` | | |

`ProjectionMatrix` offers `one()`, `zero()`, `shift(x, y, z)`, `rotate(x, y, z)`, `scale(f)`, `normalize()`,
`projected_depth(...)`, `Mul`, and `From<[[f64; 4]; 4]>`.

`Axes3dStyle` (`configure_axes()`):

| Method | Parameters | Default |
|---|---|---|
| `x_labels(n)`, `y_labels(n)`, `z_labels(n)` | `usize` | 10 each |
| `x_max_light_lines(n)`, `y_max_light_lines(n)`, `z_max_light_lines(n)`, `max_light_lines(n)` | `usize` | 10 |
| `tick_size(v)` | `SizeDesc` | `min(5, 5 % of the smaller plotting-area side)` px |
| `axis_panel_style(s)` | `Into<ShapeStyle>`: the background panels | `BLACK.mix(0.1)` |
| `bold_grid_style(s)` | `Into<ShapeStyle>` | `BLACK.mix(0.2)` |
| `light_grid_style(s)` | `Into<ShapeStyle>` | `TRANSPARENT` |
| `label_style(s)` | `Into<TextStyle>` | `("sans-serif", min(12, 12 % of the smaller side))` |
| `x_formatter(f)`, `y_formatter(f)`, `z_formatter(f)` | `&'b F`, `F: Fn(&V) -> String` | the coordinate's `ValueFormatter::format` |
| `draw()` | | |

The axis line style is fixed at `BLACK.mix(0.8)` (a field without a setter).
There are no axis descriptions and no `disable_*` options in 3D.
`draw_series` draws in iteration order without depth sorting.
Only `Cubiod` sorts its own faces.
`SurfaceSeries` is described in [section 6](#6-series).

## 11. Cargo features

Sources: `Cargo.toml.orig` of all four crates, `src/lib.rs`, and `feature = "..."` gates in `src/`.

Default set: `bitmap_backend`, `bitmap_encoder`, `bitmap_gif`, `svg_backend`, `chrono`, `ttf`, `image`,
`deprecated_items`, `all_series`, `all_elements`, `full_palette`, `colormaps`.

| Feature | Enables | Gates | Default |
|---|---|---|---|
| `bitmap_backend` | optional dep `plotters-bitmap` (no default features) | `BitMapBackend`, `BitMapElement`, `plotters::backend::{RGBPixel, BGRXPixel, PixelFormat}` | yes |
| `bitmap_encoder` | `plotters-bitmap/image_encoder`, which adds `image` 0.24 with png, jpeg, bmp | `BitMapBackend::new` and file saving. Meant to be used together with `bitmap_backend`. | yes |
| `bitmap_gif` | `plotters-bitmap/gif_backend` (`gif` 0.12 plus `image_encoder`) | `BitMapBackend::gif` | yes |
| `svg_backend` | optional dep `plotters-svg` | `SVGBackend` | yes |
| `chrono` | optional dep `chrono` 0.4.32 | `RangedDate`, `RangedDateTime`, `RangedDuration`, `Monthly`, `Yearly`, `IntoMonthly`, `IntoYearly` | yes |
| `datetime` | `chrono` | alias | no |
| `ttf` | `font-kit` 0.14.2, `ttf-parser` 0.20, `lazy_static`, `pathfinder_geometry` | system-font text engine | yes |
| `fontconfig-dlopen` | `font-kit/source-fontconfig-dlopen` | loads fontconfig at run time. Does not turn on `ttf` by itself. | no |
| `ab_glyph` | `ab_glyph` 0.2.12, `once_cell` | pure-Rust text engine and `register_font`; renders only when `ttf` is off | no |
| `image` | optional dep `image` 0.24 (png, jpeg, bmp; non-wasm) | `From<(Coord, DynamicImage)>` for `BitMapElement` | yes |
| `deprecated_items` | | `prelude::Path` | yes |
| `all_series` | `area_series`, `line_series`, `point_series`, `surface_series` | | yes |
| `all_elements` | `errorbar`, `candlestick`, `boxplot`, `histogram` | | yes |
| `line_series` | | `LineSeries`, `DashedLineSeries`, `DottedLineSeries` | via `all_series` |
| `area_series` | | `AreaSeries` | via `all_series` |
| `point_series` | | `PointSeries` | via `all_series` |
| `surface_series` | | `SurfaceSeries` | via `all_series` |
| `histogram` | | `Histogram` (a series, but grouped under `all_elements`) | via `all_elements` |
| `errorbar` | | `ErrorBar` | via `all_elements` |
| `candlestick` | | `CandleStick` | via `all_elements` |
| `boxplot` | | `Boxplot` (`Quartiles` is always available) | via `all_elements` |
| `full_palette` | | `style::full_palette` | yes |
| `colormaps` | | `style::colors::colormaps` | yes |
| `evcxr` | `svg_backend` | `evcxr_figure`, `evcxr_figure_with_saving`, `SVGWrapper` | no |
| `evcxr_bitmap` | `evcxr`, `bitmap_backend`, `plotters-svg/bitmap_encoder` | `evcxr_bitmap_figure`: renders a bitmap and embeds it in SVG as a PNG data URI | no |

- The optional dependencies also have implicit features (`plotters-bitmap`, `plotters-svg`, `font-kit`, `ttf-parser`,
  `lazy_static`, `pathfinder_geometry`, `once_cell`), except `ab_glyph`, which uses `dep:`.
- The feature table in the crate docs (`src/lib.rs`) mentions `svg`, `bitmap` and `debug`,
  which are not plotters features.
- Always available, whatever the features: `Circle`, `Cross`, `TriangleMarker`, `Rectangle`, `Polygon`, `PathElement`,
  `DashedPathElement`, `DottedPathElement`, `Pixel`, `Text`, `MultiLineText`, `Pie`, `EmptyElement`, `DynElement`,
  `Cubiod`, `Quartiles`, all numeric, slice and combinator coordinates, palettes and the named colours.
- plotters-bitmap: `default = ["image_encoder", "gif_backend"]`, `image_encoder = ["image"]`,
  `gif_backend = ["gif", "image_encoder"]`. plotters depends on it with `default-features = false`.
- plotters-svg: `bitmap_encoder = ["image"]`, `debug`.
  The `debug` code path refers to an undefined `font` variable and to `crate::prelude::RED`,
  so it does not look like it compiles.
- Build requirements with `ttf` on Linux:
  font-kit depends unconditionally on `freetype-sys` and `yeslogic-fontconfig-sys`.
  The crate docs list `pkg-config libfreetype6-dev libfontconfig1-dev` for Ubuntu.

## Closure-shaped parameters

Every place in the public API that takes a closure or function value:

| API | Signature | What it decides | Non-closure alternative in plotters |
|---|---|---|---|
| `MeshStyle::x_label_formatter`, `y_label_formatter`; the same on `SecondaryMeshStyle` | `&'b dyn Fn(&V) -> String` | tick label text | the coordinate's default `ValueFormatter` |
| `Axes3dStyle::x_formatter`, `y_formatter`, `z_formatter` | `&'b F`, `F: Fn(&V) -> String` | 3D tick label text | default `ValueFormatter` |
| `SeriesAnno::legend` | `Fn(BackendCoord) -> E + 'a` | legend glyph (any element, in px) | none. Without it an entry has no glyph. |
| `PointSeries::of_element` | `&'a F`, `F: Fn(Coord, Size, ShapeStyle) -> E` | marker element per point, often an `EmptyElement` composition | `PointSeries::new` with the marker type as a type parameter (`Circle`, `Cross`, `TriangleMarker`, `Pixel`) |
| `DottedLineSeries::new`, `DottedPathElement::new` | `Fn(BackendCoord) -> Marker + 'static` | marker drawn along the path | `DashedLineSeries` with a short dash |
| `Histogram::style_func` | `Fn(&BR::ValueType, &A) -> ShapeStyle` | per-bar style from bucket and height | `Histogram::style` (one style), or one histogram per style |
| `Histogram::baseline_func` | `Fn(&BR::ValueType) -> A` | per-bar baseline | `Histogram::baseline` (one value) |
| `SurfaceSeries::new`, `xoy`, `xoz`, `yoz` | `Fn(A, B) -> C` | the surface itself, sampled on the two iterators | none. Precomputed grids would need a lookup closure or individual `Polygon`s. |
| `SurfaceSeries::style_func` | `&'a F`, `F: Fn(&C) -> ShapeStyle` | face colour from the dependent value | `SurfaceSeries::style` |
| `ChartContext::with_projection` (3D); `Cartesian3d::with_projection`, `set_projection` | `FnOnce(ProjectionMatrixBuilder) -> ProjectionMatrix` | view angles and scale | none on `ChartContext`; the builder's `yaw`, `pitch`, `scale` are plain fields set inside the closure |
| `BuildNestedCoord::nested_coord` | `Fn(PrimaryValue) -> S`, `S: AsRangedCoord` | secondary range per category | none |
| `BindKeyPointMethod::with_key_point_func`, `with_light_point_func` | `Fn(usize) -> Vec<V> + 'static` | tick positions from the maximum count | `with_key_points(Vec<V>)` and `with_light_points(iter)`, but not for float axes in 0.3.7 |
| `DrawingArea::draw_mesh` (low-level) | `FnMut(&mut DB, MeshLine<X, Y>) -> Result` | how each grid line is drawn | `configure_mesh()` |
| `evcxr_figure`, `evcxr_figure_with_saving`, `evcxr_bitmap_figure` | `FnOnce(DrawingArea) -> Result<(), Box<dyn Error>>` | the whole drawing | |

`split_by_breakpoints` takes slices of `SizeDesc`, not closures.
Styles, palettes and colour maps are values or types.
`into_coord_trans` and friends *return* closures.

## Ordering and grouping semantics

- **`LineSeries`** collects the iterator into a `Vec` and draws one polyline through the points **in the given order**.
  It does not sort, deduplicate or break the line.
  NaN coordinates map to the start of the axis pixel range, so a NaN draws a spike to the axis rather than a gap.
  `DashedLineSeries`, `DottedLineSeries`, `AreaSeries` and `PointSeries` also keep the input order.
- **`Histogram::data`** accumulates into a `HashMap<usize, A>` keyed by the bucket's discrete index, using `+=`,
  so repeated x values **sum** their y values.
  `(x, 1)` counts rows.
  - The bar for index `i` spans from value `i` to value `i + 1` of the discrete axis.
    On a plain integer range the last value therefore gets no bar; on a segmented range it does.
  - An x that `index_of` does not know is dropped.
    Plain integer ranges do not bound-check the top end,
    so values above the range give bars that are clamped to the border.
    On a segmented range they are dropped *(verified)*.
  - Bars are emitted in `HashMap` order, which only matters when bars overlap.
    Calling `data` again replaces the earlier data.
    The baseline defaults to `A::default()`, i.e. 0.
- **`AreaSeries`** builds a polygon from the points in order,
  followed by `(last_x, baseline)` and `(first_x, baseline)`.
  The baseline is one constant y value: there is no per-point lower bound and no stacking.
  The border is drawn along the data points only and is transparent by default.
- **`Quartiles::new`** sorts its own copy.
  `CandleStick` picks its style by comparing open and close.
  Nothing else in the series and element set sorts or aggregates.
- **Ranges are never auto-fitted.**
  `build_cartesian_2d` and `build_cartesian_3d` take explicit ranges,
  and key points and labels are computed from those ranges only.
  `plotters::data::fitting_range` is an explicit helper that returns `min..max` without padding.
- **Out-of-range data is clamped, not clipped.**
  Coordinate mapping extrapolates linearly,
  then `DrawingArea::draw` clamps every mapped point to the plotting rectangle.
  A line to an out-of-range point is drawn to the border point instead
  (a point at y = 20 on a 0..10 axis lands on the border *(verified)*).
  Circles, text and other px-sized parts are anchored at the clamped point but keep their full size,
  so they can extend into the label areas.
  SVG output has no clip paths.
  The bitmap backend drops pixels outside the canvas.
- **Unknown categories** map to the start of the axis (`RangedSlice`) or to the first slot (`NestedRange`).
- **Draw order is call order.**
  The mesh, each series and the legend are painted when their call runs, so later calls draw on top.
  2D drawing does not reorder anything, and 3D drawing does not sort by depth except inside `Cubiod`.

## Changes on master since 0.3.7

`master` (HEAD `c63248e`, 2026-03-17) is 54 commits ahead of tag `v0.3.7`.
No release has been made from it.
Changes that affect the API or output:

- `ValueFormatter` for `WithKeyPoints<RangedCoordf32>` and `WithKeyPoints<RangedCoordf64>`,
  so custom key points on float axes work with `configure_mesh()` (`6268a7f`).
- A negative tick mark size now only flips the tick direction, and the axis line stays on the outer edge (`30e47b0`).
- `HSLColor`: the hue wraps (`rem_euclid(1.0)`) instead of being clamped.
  New `HSLColor::try_new(h, s, l)`, `HSLColor::from_degrees(deg, s, l)` and `HSLColorError`
  (`77b8469`, `2369300`, `398328e`).
- New `serialization` feature: serde `Serialize`/`Deserialize` on the colour types (`7f96558`).
- `LogCoord` implements `ReversibleRanged` (`95cb03b`).
  `Clone` is derived for the ranged combinators (`e4d660a`).
- `ttf`: font handle and face-parse failures return `FontError::{FontHandleUnavailable, FaceParseError}`
  instead of hitting `unreachable!()` (`1ee1d06`, `ca681a6`).
- The SVG backend writes attributes directly into the output buffer.
  The attributes emitted are the same, with small formatting differences (`627a18d` and follow-ups).
- Dependency bumps: `image` 0.25.9, `gif` 0.14, `ttf-parser` 0.25.1.

## Sources

- [plotters on crates.io](https://crates.io/crates/plotters) (latest version 0.3.7, 2024-09-08)
- [plotters 0.3.7 documentation](https://docs.rs/plotters/0.3.7/plotters/)
- [plotters-rs/plotters](https://github.com/plotters-rs/plotters), branch `master`, HEAD `c63248e`
- [font-kit 0.14.2 on crates.io](https://crates.io/crates/font-kit/0.14.2)
- Local sources: `~/.cargo/registry/src/index.crates.io-*/plotters-0.3.7`, `plotters-backend-0.3.7`,
  `plotters-bitmap-0.3.7`, `plotters-svg-0.3.7`, including `examples/` (`two-scales.rs`, `nested_coord.rs`,
  `histogram.rs`, `normal-dist.rs`, `boxplot.rs`, `tick_control.rs`, `3d-plot.rs`, `pie.rs`, `slc-temp.rs`, `stock.rs`)
- The previous duckers README: `git show 3cc964dc26a1:README.md`
