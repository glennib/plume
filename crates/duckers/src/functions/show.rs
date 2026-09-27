//! `show(chart, ...)`, which hands a rendered chart to a viewer, and `duckers_set` / `duckers_get`,
//! which change and read its process-wide defaults.

use std::sync::atomic::{AtomicU64, Ordering};

use duckers_chart::{Root, Value, image_size, to_rgb};
use duckers_view::{Image, ShowError, ShowOptions, Viewer, settings};

use super::args::{chart_error, scalar};
use crate::capi::{
    Bind, BindData, Error, Extension, InputVector, OutputVector, Result, ScalarFunction,
    ScalarInput, TypeId,
};
use crate::types::Types;

/// The bind data of one `show()` call site: how many rows it has shown.
///
/// DuckDB binds a call site once per planned statement and runs every execution of that plan with
/// the same bind data, so this counts per `show()` call in a statement, across all threads, and
/// across the executions of a prepared statement.
#[derive(Default)]
struct Site {
    shown: AtomicU64,
}

impl Site {
    /// Takes one slot under the cap, or returns `false` once `max_show` rows have been shown.
    fn take(&self, max_show: u64) -> bool {
        self.shown
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < max_show).then_some(n + 1)
            })
            .is_ok()
    }
}

/// The argument positions of `show`.
const CHART: usize = 0;
const VIEWER: usize = 1;
const WAIT: usize = 2;
const WIDTH: usize = 3;
const HEIGHT: usize = 4;

pub fn register(ext: &Extension<'_>, t: &Types) -> Result<()> {
    let ctx = ext.context();
    ext.register_scalar(
        ScalarFunction::new("show", &t.chart, exec)
            .param("chart", &t.chart)
            .named("viewer", &t.varchar, ctx.null(&t.varchar)?)
            .named("wait", &t.boolean, ctx.null(&t.boolean)?)
            .named("width", &t.bigint, ctx.null(&t.bigint)?)
            .named("height", &t.bigint, ctx.null(&t.bigint)?)
            .volatile()
            .special_nulls()
            .bind(bind),
    )?;
    ext.register_scalar(
        scalar("duckers_set", &t.varchar, |a| {
            let key = a.str(0)?;
            let v = a.arg(1);
            let value = if v.type_id() == TypeId::VARCHAR {
                v.str(a.row())?.to_string()
            } else {
                v.value(a.row())?.to_display()
            };
            settings::set(key, &value).map_err(|e| settings_error("duckers_set", e))?;
            settings::get(key).map_err(|e| settings_error("duckers_set", e))
        })
        .param("key", &t.varchar)
        .param("value", &t.any)
        .volatile(),
    )?;
    ext.register_scalar(
        scalar("duckers_get", &t.varchar, |a| {
            settings::get(a.str(0)?).map_err(|e| settings_error("duckers_get", e))
        })
        .param("key", &t.varchar)
        .volatile(),
    )
}

fn settings_error(name: &str, e: duckers_view::SettingsError) -> Error {
    Error::invalid_input(format!("{name}: {e}"))
}

/// Checks the constant arguments, so that a misspelt viewer fails before anything runs.
fn bind(b: &mut Bind<'_>) -> Result<Option<BindData>> {
    if let Ok(v) = b.arg_value(VIEWER)
        && !v.is_null()
    {
        parse_viewer(&v.as_str()?).map_err(|e| Error::binder(e.message().to_string()))?;
    }
    let side = |index: usize| -> Result<Option<i64>> {
        match b.arg_value(index) {
            Ok(v) if !v.is_null() => Ok(Some(v.as_i64()?)),
            _ => Ok(None),
        }
    };
    let (width, height) = (side(WIDTH)?, side(HEIGHT)?);
    if width.is_some() || height.is_some() {
        image_size(width, height)
            .map_err(|e| Error::binder(chart_error("show", e).message().to_string()))?;
    }
    Ok(Some(Box::new(Site::default())))
}

fn parse_viewer(name: &str) -> Result<Viewer> {
    name.parse::<Viewer>()
        .map_err(|e| Error::invalid_input(format!("show: {e}")))
}

/// A `NULL`-able argument: `None` where the row is NULL.
fn opt<'a, 'b, T>(
    v: &'a InputVector<'b>,
    row: usize,
    read: impl FnOnce(&'a InputVector<'b>, usize) -> Result<T>,
) -> Result<Option<T>> {
    if v.is_valid(row) {
        read(v, row).map(Some)
    } else {
        Ok(None)
    }
}

fn exec(input: &ScalarInput<'_>, out: &mut OutputVector<'_>) -> Result<()> {
    let site = input
        .bind_data::<Site>()
        .ok_or_else(|| Error::internal("show: missing bind data"))?;
    for row in 0..input.rows() {
        let chart = input.arg(CHART);
        if !chart.is_valid(row) {
            out.set_null(row)?;
            continue;
        }
        let bytes = chart.bytes(row)?;
        let decoded = Root::decode(bytes).map_err(|e| chart_error("show", e))?;

        // The named arguments are checked on every row, shown or not, so that a bad value does
        // not depend on the cap. A NULL named argument means its default.
        let viewer = opt(input.arg(VIEWER), row, |v, r| parse_viewer(v.str(r)?))?;
        // SAFETY: `wait` is declared BOOLEAN, stored as one byte.
        let wait = opt(input.arg(WAIT), row, |v, r| {
            Ok(unsafe { v.get::<u8>(r) } != 0)
        })?;
        let width = opt(input.arg(WIDTH), row, InputVector::i64)?;
        let height = opt(input.arg(HEIGHT), row, InputVector::i64)?;
        let (w, h) = image_size(width, height).map_err(|e| chart_error("show", e))?;

        if site.take(settings::max_show()) {
            let options = ShowOptions {
                viewer: match viewer {
                    Some(v) => v,
                    None => settings::viewer().map_err(|e| settings_error("show", e))?,
                },
                wait: match wait {
                    Some(w) => w,
                    None => settings::wait().map_err(|e| settings_error("show", e))?,
                },
            };
            let rgb = to_rgb(&decoded, w, h).map_err(|e| chart_error("show", e))?;
            let image =
                Image::from_rgb(w, h, rgb).map_err(|e| Error::internal(format!("show: {e}")))?;
            duckers_view::show(&image, &options).map_err(show_error)?;
        }
        out.set_bytes(row, bytes)?;
    }
    Ok(())
}

fn show_error(e: ShowError) -> Error {
    let message = format!("show: {e}");
    match e {
        ShowError::Image(_) => Error::internal(message),
        _ => Error::io(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_counts_across_calls() {
        let site = Site::default();
        assert!(site.take(2));
        assert!(site.take(2));
        assert!(!site.take(2));
        // Raising the cap lets more rows through; the count is kept.
        assert!(site.take(3));
        assert!(!site.take(3));
        assert!(!Site::default().take(0));
    }
}
