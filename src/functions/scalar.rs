//! Scalar functions on `CHART` values: modifiers (`CHART -> CHART`), outputs and the
//! `CHART -> VARCHAR` summary cast.
//!
//! Modifier names follow the plotters builder methods they correspond to (`caption`,
//! `x_desc`, ...), so a chain like `line_series(x, y).caption('T').x_desc('x')` reads like
//! `ChartBuilder::on(&root).caption("T", ...)`.

use super::{
    capi::{BoxError, CastImpl, Column, chart_type_handle},
    render,
    spec::{Caption, Chart, XRange},
};
use duckdb::{
    Connection,
    core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId},
    ffi,
    vscalar::{ScalarFunctionSignature, VScalar},
    vtab::arrow::WritableVector,
};

const DEFAULT_CAPTION_SIZE: u32 = 30;

fn ty(id: LogicalTypeId) -> LogicalTypeHandle {
    id.into()
}

fn sig(params: Vec<LogicalTypeHandle>, ret: LogicalTypeHandle) -> ScalarFunctionSignature {
    ScalarFunctionSignature::exact(params, ret)
}

/// Runs `f` for every input row. A NULL in any argument gives a NULL result, and `f`
/// returning `None` does too.
fn for_each_row(
    input: &mut DataChunkHandle,
    output: &mut dyn WritableVector,
    f: impl Fn(&[Column], usize) -> Result<Option<Vec<u8>>, BoxError>,
) -> Result<(), BoxError> {
    let columns = unsafe { Column::from_chunk(input.get_ptr()) };
    let mut out = output.flat_vector();
    for row in 0..input.len() {
        let result = if columns.iter().all(|c| c.is_valid(row)) {
            f(&columns, row)?
        } else {
            None
        };
        match result {
            Some(bytes) => out.insert(row, bytes.as_slice()),
            None => out.set_null(row),
        }
    }
    Ok(())
}

fn chart_at(columns: &[Column], row: usize) -> Result<Chart, BoxError> {
    Ok(Chart::decode(unsafe { columns[0].bytes(row) })?)
}

fn string_at(columns: &[Column], index: usize, row: usize) -> String {
    unsafe { columns[index].string(row) }
}

fn u32_at(columns: &[Column], index: usize, row: usize, what: &str) -> Result<u32, BoxError> {
    let value: i32 = unsafe { columns[index].get(row) };
    u32::try_from(value).map_err(|_| format!("{what} must not be negative, got {value}").into())
}

/// A modifier: decodes the chart in column 0, applies `f`, re-encodes it.
fn modify(
    input: &mut DataChunkHandle,
    output: &mut dyn WritableVector,
    f: impl Fn(&mut Chart, &[Column], usize) -> Result<(), BoxError>,
) -> Result<(), BoxError> {
    for_each_row(input, output, |columns, row| {
        let mut chart = chart_at(columns, row)?;
        f(&mut chart, columns, row)?;
        Ok(Some(chart.encode()))
    })
}

/// `caption(chart, text [, size])`, after `ChartBuilder::caption`.
struct CaptionFn;

impl VScalar for CaptionFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        let sized = input.num_columns() == 3;
        modify(input, output, |chart, columns, row| {
            let size = if sized {
                u32_at(columns, 2, row, "caption size")?
            } else {
                DEFAULT_CAPTION_SIZE
            };
            chart.caption = Some(Caption {
                text: string_at(columns, 1, row),
                size,
            });
            Ok(())
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![
            sig(
                vec![chart_type_handle(), ty(LogicalTypeId::Varchar)],
                chart_type_handle(),
            ),
            sig(
                vec![
                    chart_type_handle(),
                    ty(LogicalTypeId::Varchar),
                    ty(LogicalTypeId::Integer),
                ],
                chart_type_handle(),
            ),
        ]
    }
}

/// `x_desc(chart, text)`, after `MeshStyle::x_desc`.
struct XDescFn;

impl VScalar for XDescFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        modify(input, output, |chart, columns, row| {
            chart.x_desc = Some(string_at(columns, 1, row));
            Ok(())
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![sig(
            vec![chart_type_handle(), ty(LogicalTypeId::Varchar)],
            chart_type_handle(),
        )]
    }
}

/// `y_desc(chart, text)`, after `MeshStyle::y_desc`.
struct YDescFn;

impl VScalar for YDescFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        modify(input, output, |chart, columns, row| {
            chart.y_desc = Some(string_at(columns, 1, row));
            Ok(())
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![sig(
            vec![chart_type_handle(), ty(LogicalTypeId::Varchar)],
            chart_type_handle(),
        )]
    }
}

fn check_range<T: PartialOrd + std::fmt::Debug>(lo: T, hi: T) -> Result<(), BoxError> {
    if lo < hi {
        Ok(())
    } else {
        Err(format!("range start must be below its end, got {lo:?} and {hi:?}").into())
    }
}

/// `x_range(chart, lo, hi)`: the x-axis range passed to `build_cartesian_2d`. `DOUBLE` bounds
/// for numeric axes, `TIMESTAMP` bounds for time axes.
struct XRangeFn;

impl VScalar for XRangeFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        let time = input.flat_vector(1).logical_type().id() == LogicalTypeId::Timestamp;
        modify(input, output, |chart, columns, row| {
            chart.x_range = Some(unsafe {
                if time {
                    let (lo, hi) = (columns[1].get::<i64>(row), columns[2].get::<i64>(row));
                    check_range(lo, hi)?;
                    XRange::Time(lo, hi)
                } else {
                    let (lo, hi) = (columns[1].get::<f64>(row), columns[2].get::<f64>(row));
                    check_range(lo, hi)?;
                    XRange::F64(lo, hi)
                }
            });
            Ok(())
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        [LogicalTypeId::Double, LogicalTypeId::Timestamp]
            .into_iter()
            .map(|bound| {
                sig(
                    vec![chart_type_handle(), ty(bound), ty(bound)],
                    chart_type_handle(),
                )
            })
            .collect()
    }
}

/// `y_range(chart, lo, hi)`: the y-axis range passed to `build_cartesian_2d`.
struct YRangeFn;

impl VScalar for YRangeFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        modify(input, output, |chart, columns, row| {
            let (lo, hi) = unsafe { (columns[1].get::<f64>(row), columns[2].get::<f64>(row)) };
            check_range(lo, hi)?;
            chart.y_range = Some((lo, hi));
            Ok(())
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![sig(
            vec![
                chart_type_handle(),
                ty(LogicalTypeId::Double),
                ty(LogicalTypeId::Double),
            ],
            chart_type_handle(),
        )]
    }
}

/// `draw_series(a, b)`: the series of `b` drawn on top of `a`, on `a`'s settings. Like calling
/// `draw_series` again on the same `ChartContext`.
struct DrawSeriesFn;

impl VScalar for DrawSeriesFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        modify(input, output, |chart, columns, row| {
            let other = Chart::decode(unsafe { columns[1].bytes(row) })?;
            chart.series.extend(other.series);
            chart.axis_kind()?;
            Ok(())
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![sig(
            vec![chart_type_handle(), chart_type_handle()],
            chart_type_handle(),
        )]
    }
}

fn size_at(columns: &[Column], row: usize) -> Result<(u32, u32), BoxError> {
    if columns.len() == 3 {
        Ok((
            u32_at(columns, 1, row, "width")?,
            u32_at(columns, 2, row, "height")?,
        ))
    } else {
        Ok(render::DEFAULT_SIZE)
    }
}

fn output_signatures(ret: LogicalTypeId) -> Vec<ScalarFunctionSignature> {
    vec![
        sig(vec![chart_type_handle()], ty(ret)),
        sig(
            vec![
                chart_type_handle(),
                ty(LogicalTypeId::Integer),
                ty(LogicalTypeId::Integer),
            ],
            ty(ret),
        ),
    ]
}

/// `to_svg(chart [, width, height])`: renders with `SVGBackend`.
struct ToSvgFn;

impl VScalar for ToSvgFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        for_each_row(input, output, |columns, row| {
            let chart = chart_at(columns, row)?;
            Ok(Some(
                render::to_svg(&chart, size_at(columns, row)?)?.into_bytes(),
            ))
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        output_signatures(LogicalTypeId::Varchar)
    }
}

/// `to_png(chart [, width, height])`: renders with `BitMapBackend` and encodes a PNG.
struct ToPngFn;

impl VScalar for ToPngFn {
    type State = ();

    fn invoke(
        _: &(),
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        for_each_row(input, output, |columns, row| {
            let chart = chart_at(columns, row)?;
            Ok(Some(render::to_png(&chart, size_at(columns, row)?)?))
        })
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        output_signatures(LogicalTypeId::Blob)
    }
}

/// `CAST(chart AS VARCHAR)`: a short summary, which is also how the CLI shows a `CHART`.
pub struct SummaryCast;

impl CastImpl for SummaryCast {
    fn cast(count: usize, input: &Column, output: &mut ffi::duckdb_vector) -> Result<(), BoxError> {
        let mut out = output.flat_vector();
        for row in 0..count {
            if input.is_valid(row) {
                let chart = Chart::decode(unsafe { input.bytes(row) })?;
                out.insert(row, chart.summary().as_str());
            } else {
                out.set_null(row);
            }
        }
        Ok(())
    }
}

pub fn register(con: &Connection) -> Result<(), BoxError> {
    con.register_scalar_function::<CaptionFn>("caption")?;
    con.register_scalar_function::<XDescFn>("x_desc")?;
    con.register_scalar_function::<YDescFn>("y_desc")?;
    con.register_scalar_function::<XRangeFn>("x_range")?;
    con.register_scalar_function::<YRangeFn>("y_range")?;
    con.register_scalar_function::<DrawSeriesFn>("draw_series")?;
    con.register_scalar_function::<ToSvgFn>("to_svg")?;
    con.register_scalar_function::<ToPngFn>("to_png")?;
    Ok(())
}
