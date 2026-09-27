//! Render tests: the plan's worked examples that M2 covers, built with the calls the SQL layer
//! makes, plus the axis kinds and edge cases, and M4's styling (mesh, legend, log scales,
//! horizontal histograms, markers, builder sizes) as `test/svg/` renders it from SQL.
//!
//! Set `DUCKERS_CHART_PNG_DIR` to also write every chart as a PNG into that directory.

use duckers_chart::spec::MeshSetting;
use duckers_chart::{
    Accumulator, Chart, Font, Key, RangeValue, Series, SeriesAggregate, SeriesBinding, SortKey,
    SqlType, Value, XValue, mix, to_png, to_rgb, to_svg,
};

/// One row of an aggregate call.
struct Row {
    x: XValue,
    y: f64,
    key: Option<Key>,
    order_by: Option<SortKey>,
}

fn row(x: XValue, y: f64) -> Row {
    Row {
        x,
        y,
        key: None,
        order_by: None,
    }
}

/// Runs an aggregate over rows the way DuckDB would with two threads: two partial states,
/// combined, then finalized.
fn aggregate(aggregate: SeriesAggregate, x: SqlType, rows: Vec<Row>) -> Vec<Series> {
    let binding = SeriesBinding::new(aggregate, x).unwrap();
    let keyed = rows.iter().any(|r| r.key.is_some());
    let mut parts = [Accumulator::new(), Accumulator::new()];
    for (i, r) in rows.into_iter().enumerate() {
        parts[i % 2]
            .push(&binding, Some(r.x), Some(r.y), r.key, r.order_by)
            .unwrap();
    }
    let [mut a, b] = parts;
    a.combine(b);
    if keyed {
        a.finish_keyed(&binding).unwrap()
    } else {
        vec![a.finish(&binding).unwrap()]
    }
}

fn one(aggregate_kind: SeriesAggregate, x: SqlType, rows: Vec<Row>) -> Series {
    aggregate(aggregate_kind, x, rows).pop().unwrap()
}

/// DATE '2024-01-01' as days since the epoch.
const JAN_1_2024: i32 = 19723;

/// A deterministic temperature curve per city.
fn weather(city: usize) -> Vec<(i32, f64)> {
    (0..60)
        .map(|d| {
            let t = d as f64;
            let temp = -2.0 + 4.0 * city as f64 + 0.3 * t + 5.0 * (t / 5.0 + city as f64).sin();
            (JAN_1_2024 + d, (temp * 10.0).round() / 10.0)
        })
        .collect()
}

/// `SELECT chart().caption('Oslo temperature', 30).margin(10).x_label_area_size(30)
///  .y_label_area_size(40).y_range(-10, 30).configure_mesh().x_desc('day').y_desc('°C').draw()
///  .draw_series(line_series(day, temp).style('red').label('temp'))
///  .configure_series_labels().draw() FROM weather WHERE city = 'Oslo'`
///
/// The margin and label area sizes are duckers' defaults and `border_style` is left out, so the
/// snapshot stays the core chart; `test/svg/oslo.sql` renders the full example from SQL.
fn oslo() -> Chart {
    let rows = weather(0)
        .into_iter()
        .map(|(d, t)| row(XValue::Date(d), t))
        .collect();
    let temp = one(SeriesAggregate::LineSeries, SqlType::Date, rows)
        .style("red", None)
        .unwrap()
        .label("temp");
    Chart::new()
        .caption("Oslo temperature", Some(30))
        .unwrap()
        .y_range(RangeValue::Number(-10.0), RangeValue::Number(30.0))
        .unwrap()
        .configure_mesh()
        .x_desc("day")
        .y_desc("°C")
        .draw()
        .draw_series(temp)
        .unwrap()
        .configure_series_labels()
        .draw()
}

/// `SELECT chart().draw_series(line_series(day, temp, key := city)) FROM weather`
fn cities() -> Chart {
    let mut rows = Vec::new();
    for (i, city) in ["Oslo", "Bergen", "Tromsø"].into_iter().enumerate() {
        for (d, t) in weather(i) {
            rows.push(Row {
                key: Some(Key {
                    sort: SortKey::Text(city.into()),
                    label: Some(city.into()),
                }),
                ..row(XValue::Date(d), t)
            });
        }
    }
    let series = aggregate(SeriesAggregate::LineSeries, SqlType::Date, rows);
    Chart::new().draw_series_list(series).unwrap()
}

/// `SELECT chart().caption('Articles per section')
///  .draw_series(histogram(section, n, order_by := -n).style('blue_400')) FROM (...)`
fn articles() -> Chart {
    let sections = [
        ("sport", 412.0),
        ("culture", 158.0),
        ("news", 530.0),
        ("economy", 201.0),
        ("opinion", 97.0),
    ];
    let rows = sections
        .into_iter()
        .map(|(s, n)| Row {
            order_by: Some(SortKey::Float(-n)),
            ..row(XValue::Category(s.into()), n)
        })
        .collect();
    let bars = one(SeriesAggregate::Histogram, SqlType::Varchar, rows)
        .style("blue_400", None)
        .unwrap();
    Chart::new()
        .caption("Articles per section", None)
        .unwrap()
        .draw_series(bars)
        .unwrap()
}

/// `SELECT chart().configure_mesh().x_desc('x').y_desc('y').draw()
///  .draw_series(line_series(i, i * i).style('red').label('y = x²'))
///  .draw_series(point_series(i, i * i).style('black').size(4).filled())
///  .configure_series_labels().position('upper_left').draw()
///  .to_png(800, 600) FROM range(10) t(i)`
///
/// `background_style(mix('white', 0.8))` is left out; `test/svg/squares.sql` renders the full
/// example from SQL.
fn squares() -> Chart {
    let rows = || {
        (0..10)
            .map(|i| row(XValue::Integer(i), (i * i) as f64))
            .collect()
    };
    let line = one(SeriesAggregate::LineSeries, SqlType::Integer, rows())
        .style("red", None)
        .unwrap()
        .label("y = x²");
    let points = one(SeriesAggregate::PointSeries, SqlType::Integer, rows())
        .style("black", None)
        .unwrap()
        .size(4)
        .unwrap()
        .filled()
        .unwrap();
    Chart::new()
        .configure_mesh()
        .x_desc("x")
        .y_desc("y")
        .draw()
        .draw_series(line)
        .unwrap()
        .draw_series(points)
        .unwrap()
        .configure_series_labels()
        .position("upper_left")
        .unwrap()
        .draw()
}

/// `SELECT chart().draw_series(line_series(cos(t), sin(t), order_by := t))
///  FROM (SELECT i / 20.0 AS t FROM range(126) r(i))`
fn circle() -> Chart {
    let rows = (0..126)
        .map(|i| {
            let t = i as f64 / 20.0;
            Row {
                order_by: Some(SortKey::Float(t)),
                ..row(XValue::Number(t.cos()), t.sin())
            }
        })
        .collect();
    let curve = one(SeriesAggregate::LineSeries, SqlType::Float, rows);
    Chart::new().draw_series(curve).unwrap()
}

/// A timestamp axis with points and a line, and an explicit fill.
fn timestamps() -> Chart {
    let hour = 3_600_000_000i64;
    let start = i64::from(JAN_1_2024) * 24 * hour;
    let rows = || {
        (0..48)
            .map(|h| {
                let v = (h as f64 / 6.0).sin() * 10.0 + 20.0;
                row(XValue::Timestamp(start + h * hour), v)
            })
            .collect()
    };
    let line = one(SeriesAggregate::LineSeries, SqlType::Timestamp, rows())
        .label("load")
        .point_size(2)
        .unwrap();
    let points = one(SeriesAggregate::PointSeries, SqlType::Timestamp, rows()).label("samples");
    Chart::new()
        .fill("grey_100")
        .unwrap()
        .draw_series(line)
        .unwrap()
        .draw_series(points)
        .unwrap()
}

/// Integer buckets from `histogram_vertical(x, 1)`, counting rows, on a segmented integer axis.
fn dice() -> Chart {
    let rows = (0..200)
        .map(|i: i64| row(XValue::Integer((i * 7 + i / 3) % 6 + 1 + (i % 5) / 4), 1.0))
        .collect();
    Chart::new()
        .draw_series(one(SeriesAggregate::Histogram, SqlType::Integer, rows).label("rolls"))
        .unwrap()
}

/// Date buckets, one band per day, with a line through the band centres on the same axis.
fn daily() -> Chart {
    let rows = || {
        (0..14)
            .map(|d| row(XValue::Date(JAN_1_2024 + d), ((d * 37) % 11) as f64))
            .collect()
    };
    let bars = one(SeriesAggregate::Histogram, SqlType::Date, rows()).label("count");
    let line = one(SeriesAggregate::LineSeries, SqlType::Date, rows())
        .style("black", Some(2))
        .unwrap()
        .label("trend");
    Chart::new()
        .draw_series(bars)
        .unwrap()
        .draw_series(line)
        .unwrap()
}

/// A reversed x axis and a y range narrower than the data, whose points are clamped.
fn reversed() -> Chart {
    let rows = (0..20)
        .map(|i| row(XValue::Number(i as f64), (i as f64 - 10.0).powi(2)))
        .collect();
    Chart::new()
        .x_range(RangeValue::Number(20.0), RangeValue::Number(0.0))
        .unwrap()
        .y_range(RangeValue::Number(0.0), RangeValue::Number(50.0))
        .unwrap()
        .draw_series(one(SeriesAggregate::LineSeries, SqlType::Float, rows))
        .unwrap()
}

/// `test/svg/mesh_styled.sql`: every kind of mesh setting, formatters on a date and a numeric
/// axis.
fn mesh_styled() -> Chart {
    let rows = weather(0)
        .into_iter()
        .map(|(d, t)| row(XValue::Date(d), t))
        .collect();
    let temp = one(SeriesAggregate::LineSeries, SqlType::Date, rows)
        .style("red", Some(2))
        .unwrap();
    Chart::new()
        .caption_font(
            "Oslo, styled mesh",
            Font::new("serif", 24, Some("bold")).unwrap(),
        )
        .x_label_area_size(50)
        .unwrap()
        .y_label_area_size(80)
        .unwrap()
        .configure_mesh()
        .x_desc("day")
        .y_desc("temperature")
        .x_label_formatter("%b %d")
        .unwrap()
        .y_label_formatter("{:+.1f} °C")
        .unwrap()
        .x_labels(6)
        .unwrap()
        .y_labels(8)
        .unwrap()
        .label_style(
            Font::new("sans-serif", 11, None)
                .unwrap()
                .color("grey_700")
                .unwrap(),
        )
        .axis_desc_style(Font::new("serif", 14, Some("italic")).unwrap())
        .light_line_style("blue_50", None)
        .unwrap()
        .bold_line_style("blue_200", Some(1))
        .unwrap()
        .axis_style("blue_900", Some(2))
        .unwrap()
        .set(MeshSetting::DisableXMesh)
        .set_tick_mark_size("bottom", 8)
        .unwrap()
        .set_tick_mark_size("left", 3)
        .unwrap()
        .x_label_offset(4)
        .unwrap()
        .y_label_offset(-3)
        .unwrap()
        .draw()
        .draw_series(temp)
        .unwrap()
}

/// `test/svg/legend_styled.sql`: a legend with every option and a coloured FONT, over the
/// three cities' lines only (the SQL adds sample points and shows one month).
fn legend_styled() -> Chart {
    let lines: Vec<Series> = cities()
        .series()
        .map(|s| s.clone().stroke_width(2).unwrap())
        .collect();
    Chart::new()
        .draw_series_list(lines)
        .unwrap()
        .configure_series_labels()
        .position_at(20, 10)
        .unwrap()
        .margin(8)
        .unwrap()
        .legend_area_size(40)
        .unwrap()
        .border_style("grey_600", Some(2))
        .unwrap()
        .background_style(&mix("white", 0.85).unwrap())
        .unwrap()
        .label_font(
            Font::new("serif", 16, Some("italic"))
                .unwrap()
                .color("bluegrey_800")
                .unwrap(),
        )
        .draw()
}

/// `test/svg/log_scales.sql`: log scales on both axes.
fn log_scales() -> Chart {
    let xs: Vec<f64> = (0..31).map(|i| 10f64.powf(f64::from(i) / 10.0)).collect();
    let rows = |p: i32| {
        xs.iter()
            .map(|x| row(XValue::Number(*x), x.powi(p)))
            .collect()
    };
    let square = one(SeriesAggregate::LineSeries, SqlType::Float, rows(2)).label("x²");
    let cube = one(SeriesAggregate::PointSeries, SqlType::Float, rows(3))
        .marker("triangle")
        .unwrap()
        .size(4)
        .unwrap()
        .label("x³");
    Chart::new()
        .caption("x² and x³ on log axes", Some(20))
        .unwrap()
        .x_log_scale(None)
        .unwrap()
        .y_log_scale(None)
        .unwrap()
        .configure_mesh()
        .x_desc("x")
        .draw()
        .draw_series(square)
        .unwrap()
        .draw_series(cube)
        .unwrap()
        .configure_series_labels()
        .position("upper_left")
        .unwrap()
        .border_style("black", None)
        .unwrap()
        .draw()
}

/// `test/svg/histogram_horizontal.sql`: horizontal bars on a category axis with a margin and
/// a baseline some counts are below.
fn histogram_horizontal() -> Chart {
    let sections = [
        ("sport", 412.0),
        ("culture", 158.0),
        ("news", 530.0),
        ("economy", 201.0),
        ("opinion", 97.0),
    ];
    let rows = sections
        .into_iter()
        .map(|(s, n)| Row {
            order_by: Some(SortKey::Float(n)),
            ..row(XValue::Category(s.into()), n)
        })
        .collect();
    let bars = one(SeriesAggregate::HistogramHorizontal, SqlType::Varchar, rows)
        .style("teal_400", None)
        .unwrap()
        .margin(2)
        .unwrap()
        .baseline(150.0)
        .unwrap();
    Chart::new()
        .caption("Articles per section", Some(20))
        .unwrap()
        .y_label_area_size(70)
        .unwrap()
        .configure_mesh()
        .x_desc("articles")
        .x_label_formatter("{:.0f}")
        .unwrap()
        .draw()
        .draw_series(bars)
        .unwrap()
}

/// `test/svg/markers.sql`: the four point markers.
fn markers() -> Chart {
    let names = ["circle", "cross", "pixel", "triangle"];
    let colors = ["red", "green_700", "black", "blue_600"];
    let mut rows = Vec::new();
    for (j, name) in names.into_iter().enumerate() {
        for i in 0..20 {
            rows.push(Row {
                key: Some(Key {
                    sort: SortKey::Text(name.into()),
                    label: Some(name.into()),
                }),
                ..row(XValue::Integer(i), j as f64 + (i as f64 / 3.0).sin())
            });
        }
    }
    let series = aggregate(SeriesAggregate::PointSeries, SqlType::Integer, rows)
        .into_iter()
        .zip(names.into_iter().zip(colors))
        .map(|(s, (name, color))| {
            s.marker(name)
                .unwrap()
                .style(color, None)
                .unwrap()
                .size(5)
                .unwrap()
                .filled()
                .unwrap()
        });
    Chart::new()
        .caption("marker()", Some(20))
        .unwrap()
        .draw_series_list(series)
        .unwrap()
        .configure_series_labels()
        .position("lower_right")
        .unwrap()
        .border_style("black", None)
        .unwrap()
        .background_style("white")
        .unwrap()
        .draw()
}

/// `test/svg/builder_sizes.sql`: every ChartBuilder size method.
fn builder_sizes() -> Chart {
    let rows = (0..11)
        .map(|i| row(XValue::Integer(i), (i * i) as f64))
        .collect();
    Chart::new()
        .caption("ChartBuilder sizes", Some(20))
        .unwrap()
        .margin_top(5)
        .unwrap()
        .margin_bottom(20)
        .unwrap()
        .margin_left(30)
        .unwrap()
        .margin_right(15)
        .unwrap()
        .set_all_label_area_size(35)
        .unwrap()
        .top_x_label_area_size(45)
        .unwrap()
        .right_y_label_area_size(65)
        .unwrap()
        .set_left_and_bottom_label_area_size(50)
        .unwrap()
        .configure_mesh()
        .x_desc("x")
        .y_desc("y")
        .draw()
        .draw_series(
            one(SeriesAggregate::LineSeries, SqlType::Integer, rows)
                .style("purple", Some(2))
                .unwrap(),
        )
        .unwrap()
}

/// All charts, with the size they are rendered at.
fn charts() -> Vec<(&'static str, Chart, (u32, u32))> {
    vec![
        ("oslo", oslo(), (800, 600)),
        ("cities", cities(), (640, 480)),
        ("articles", articles(), (640, 480)),
        ("squares", squares(), (800, 600)),
        ("circle", circle(), (640, 480)),
        ("timestamps", timestamps(), (640, 480)),
        ("dice", dice(), (640, 480)),
        ("daily", daily(), (640, 480)),
        ("reversed", reversed(), (640, 480)),
        ("empty", Chart::new(), (640, 480)),
        ("mesh_styled", mesh_styled(), (800, 500)),
        ("legend_styled", legend_styled(), (640, 480)),
        ("log_scales", log_scales(), (640, 480)),
        ("histogram_horizontal", histogram_horizontal(), (640, 480)),
        ("markers", markers(), (640, 480)),
        ("builder_sizes", builder_sizes(), (640, 480)),
    ]
}

#[test]
fn write_pngs_when_asked() {
    let Some(dir) = std::env::var_os("DUCKERS_CHART_PNG_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, chart, (w, h)) in charts() {
        std::fs::write(
            dir.join(format!("{name}.png")),
            to_png(&chart, w, h).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("{name}.svg")),
            to_svg(&chart, w, h).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn png_header_and_dimensions() {
    for (name, chart, (w, h)) in charts() {
        let png = to_png(&chart, w, h).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{name}");
        assert_eq!(&png[12..16], b"IHDR", "{name}");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((width, height), (w, h), "{name}");
        // 8-bit RGB.
        assert_eq!((png[24], png[25]), (8, 2), "{name}");
    }
}

#[test]
fn rendering_is_deterministic() {
    for (name, chart, (w, h)) in charts() {
        assert_eq!(to_png(&chart, w, h), to_png(&chart, w, h), "{name}");
        assert_eq!(to_svg(&chart, w, h), to_svg(&chart, w, h), "{name}");
        let decoded = Chart::decode(&chart.encode()).unwrap();
        assert_eq!(to_png(&decoded, w, h), to_png(&chart, w, h), "{name}");
    }
}

#[test]
fn rgb_buffer_matches_png_size() {
    let rgb = to_rgb(&squares(), 320, 200).unwrap();
    assert_eq!(rgb.len(), 320 * 200 * 3);
    // The default fill is white, and the top-left corner is inside the margin.
    assert_eq!(&rgb[..3], &[255, 255, 255]);
}

#[test]
fn transparent_fill_leaves_the_bitmap_black() {
    let chart = Chart::new().fill("transparent").unwrap();
    let rgb = to_rgb(&chart, 10, 10).unwrap();
    assert_eq!(&rgb[..3], &[0, 0, 0]);
}

#[test]
fn size_limits() {
    let chart = Chart::new();
    assert!(to_png(&chart, 0, 10).is_err());
    assert!(to_svg(&chart, 10, 8193).is_err());
    assert!(to_svg(&chart, 8192, 1).is_ok());
    assert_eq!(duckers_chart::image_size(None, None).unwrap(), (640, 480));
    let err = duckers_chart::image_size(Some(-1), None).unwrap_err();
    assert_eq!(
        err.message(),
        "chart size must be between 1 and 8192 px per side, got -1x480"
    );
}

#[test]
fn svg_content() {
    let svg = to_svg(&oslo(), 800, 600).unwrap();
    assert!(svg.starts_with("<svg"));
    for text in ["Oslo temperature", "day", "°C", "temp", "2024-01-01"] {
        assert!(svg.contains(text), "missing {text}");
    }
    // The line is drawn in the style's red, not the palette.
    assert!(svg.contains("stroke=\"#FF0000\""));
    let svg = to_svg(&articles(), 640, 480).unwrap();
    // Categories in order_by order: news has the largest count.
    let news = svg.find("\nnews\n").unwrap();
    let sport = svg.find("\nsport\n").unwrap();
    let opinion = svg.find("\nopinion\n").unwrap();
    assert!(news < sport && sport < opinion);
}

#[test]
fn default_mesh_is_drawn_before_the_first_series() {
    // The mesh's axis line is black; the series is palette colour 0 (#E6194B).
    let svg = to_svg(&circle(), 640, 480).unwrap();
    let axis = svg.find("stroke=\"#000000\"").unwrap();
    let series = svg.find("stroke=\"#E6194B\"").unwrap();
    assert!(axis < series);
}

#[test]
fn automatic_legend_for_labelled_series() {
    let labelled = Chart::new()
        .draw_series(
            circle()
                .series()
                .next()
                .unwrap()
                .clone()
                .label("unit circle"),
        )
        .unwrap();
    assert!(to_svg(&labelled, 640, 480).unwrap().contains("unit circle"));
}

#[test]
fn mix_is_usable_as_a_style() {
    let series = circle()
        .series()
        .next()
        .unwrap()
        .clone()
        .style(&mix("blue", 0.5).unwrap(), Some(3))
        .unwrap();
    let svg = to_svg(&Chart::new().draw_series(series).unwrap(), 200, 200).unwrap();
    assert!(svg.contains("stroke=\"#0000FF\""));
    assert!(svg.contains("opacity=\"0.5\""));
}

macro_rules! snapshot {
    ($name:ident) => {
        #[test]
        fn $name() {
            let (_, chart, (w, h)) = charts()
                .into_iter()
                .find(|(n, _, _)| *n == stringify!($name))
                .unwrap();
            insta::assert_snapshot!(
                concat!("svg_", stringify!($name)),
                to_svg(&chart, w, h).unwrap()
            );
        }
    };
}

mod snapshots {
    use super::*;

    snapshot!(oslo);
    snapshot!(cities);
    snapshot!(articles);
    snapshot!(squares);
    snapshot!(circle);
    snapshot!(timestamps);
    snapshot!(dice);
    snapshot!(daily);
    snapshot!(reversed);
    snapshot!(empty);
    snapshot!(mesh_styled);
    snapshot!(legend_styled);
    snapshot!(log_scales);
    snapshot!(histogram_horizontal);
    snapshot!(markers);
    snapshot!(builder_sizes);
}

#[test]
fn formatted_labels() {
    let svg = to_svg(&mesh_styled(), 800, 500).unwrap();
    for label in ["\nJan 15\n", "\n+5.0 °C\n", "\nFeb 12\n"] {
        assert!(svg.contains(label), "missing {label:?}");
    }
    let svg = to_svg(&log_scales(), 640, 480).unwrap();
    for label in ["\n1\n", "\n1000\n", "\n1e6\n", "\n1e9\n"] {
        assert!(svg.contains(label), "missing {label:?}");
    }
    assert!(!svg.contains("1000000.0"), "LogCoord's {{:?}} labels");
}

#[test]
fn horizontal_bars_run_from_the_baseline() {
    // In the band order news is on top, so its bar's rect comes last; each bar starts at the
    // baseline's x and opinion (97) ends left of it.
    let svg = to_svg(&histogram_horizontal(), 640, 480).unwrap();
    let rects: Vec<(i32, i32)> = svg
        .lines()
        .filter(|l| l.starts_with("<rect") && l.contains("#26A69A"))
        .map(|l| {
            let attr = |name: &str| -> i32 {
                let start = l.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
                l[start..].split('"').next().unwrap().parse().unwrap()
            };
            (attr("x"), attr("width"))
        })
        .collect();
    assert_eq!(rects.len(), 5, "{svg}");
    let (opinion, others) = rects.split_first().unwrap();
    let baseline_x = opinion.0 + opinion.1;
    for (x, _) in others {
        assert_eq!(*x, baseline_x);
    }
}

#[test]
fn log_axis_errors_name_the_fix() {
    let bars = one(
        SeriesAggregate::Histogram,
        SqlType::Varchar,
        vec![row(XValue::Category("a".into()), 10.0)],
    );
    let chart = Chart::new()
        .y_log_scale(None)
        .unwrap()
        .draw_series(bars)
        .unwrap();
    let err = to_svg(&chart, 100, 100).unwrap_err();
    assert!(err.message().contains("a positive baseline(v)"), "{err}");
}

#[test]
fn every_font_family_renders() {
    // A family no system has still renders, with the embedded font, in PNG and SVG.
    let chart = Chart::new()
        .caption_font("t", Font::new("No Such Family", 20, Some("bold")).unwrap())
        .configure_mesh()
        .label_style(Font::new("Another Missing One", 9, None).unwrap())
        .draw()
        .configure_series_labels()
        .label_font(Font::new("Third", 9, None).unwrap())
        .draw();
    assert!(to_png(&chart, 200, 200).is_ok());
    assert!(
        to_svg(&chart, 200, 200)
            .unwrap()
            .contains("font-family=\"Another Missing One\"")
    );
}
