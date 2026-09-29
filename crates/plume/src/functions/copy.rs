//! The copy functions `png` and `svg`: `COPY (SELECT <chart>) TO 'f.png' (FORMAT png)`.
//!
//! Each output file holds exactly one chart, so the query gives one row per file: one row for a
//! plain `COPY`, one row per partition with `PARTITION_BY`. The chart is rendered and the file
//! written in the finalize step, with `std::fs`, since the C API gives a copy function a path but
//! no file-system handle. Remote paths (`s3://...`) are refused at bind time for that reason.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use plume_chart::{Root, Value, image_size, to_png, to_svg};

use super::args::chart_error;
use crate::capi::{CopyBind, CopyChunk, CopyTo, Error, Extension, LogicalType, Result, TypeId};
use crate::types::Types;

pub fn register(ext: &Extension<'_>, t: &Types) -> Result<()> {
    for format in [Format::Png, Format::Svg] {
        ext.register_copy(
            format.name(),
            ChartCopy {
                format,
                chart: t.chart.clone(),
            },
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Png,
    Svg,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
        }
    }

    /// The prefix of every error message, e.g. `FORMAT png`.
    fn label(self) -> String {
        format!("FORMAT {}", self.name())
    }

    fn render(self, chart: &Root, width: u32, height: u32) -> plume_chart::Result<Vec<u8>> {
        match self {
            Format::Png => to_png(chart, width, height),
            Format::Svg => to_svg(chart, width, height).map(String::into_bytes),
        }
    }
}

/// One of the two copy functions; its user data.
struct ChartCopy {
    format: Format,
    chart: LogicalType,
}

/// The options of one `COPY` statement.
struct Options {
    width: u32,
    height: u32,
}

/// One output file: the path DuckDB assigned, and the row it received.
struct ChartFile {
    path: String,
    /// Rows flushed to this file so far.
    rows: AtomicUsize,
    /// The chart bytes of its row, `Some(None)` if that row was NULL.
    chart: Mutex<Option<Option<Vec<u8>>>>,
}

/// The rows of one batch: how many, and the first one's chart bytes. A batch with more than one
/// row is an error once it reaches its file, so the other rows are not kept.
#[derive(Default)]
struct ChartBatch {
    rows: usize,
    first: Mutex<Option<Option<Vec<u8>>>>,
}

impl CopyTo for ChartCopy {
    type BindData = Options;
    type File = ChartFile;
    type Batch = ChartBatch;

    fn bind(&self, b: &CopyBind<'_>) -> Result<Options> {
        let label = self.format.label();
        let path = b.file_path()?;
        if local_path(&path).is_none() {
            return Err(Error::binder(format!(
                "{label} writes local files only, and '{path}' is a remote path; write to a local \
                 file, or use to_{}() with FORMAT blob, which goes through DuckDB's file system",
                self.format.name()
            )));
        }
        self.check_columns(b)?;

        let (mut width, mut height) = (None, None);
        for (name, value) in b.options()? {
            let slot = match name.to_ascii_lowercase().as_str() {
                "width" => &mut width,
                "height" => &mut height,
                _ => {
                    return Err(Error::binder(format!(
                        "{label}: unknown option {}; the options are WIDTH and HEIGHT",
                        name.to_ascii_uppercase()
                    )));
                }
            };
            let is_integer = value.logical_type().is_ok_and(|t| t.id().is_integer());
            if !is_integer {
                return Err(Error::binder(format!(
                    "{label}: {} must be an integer number of px, got {}",
                    name.to_ascii_uppercase(),
                    value.to_display()
                )));
            }
            *slot = Some(value.as_i64()?);
        }
        let (width, height) = image_size(width, height)
            .map_err(|e| Error::binder(chart_error(&label, e).message().to_string()))?;
        Ok(Options { width, height })
    }

    fn init(&self, _options: &Options, path: &str) -> Result<ChartFile> {
        // DuckDB names partition files `data_0.<extension>`, with the extension a copy function
        // declares, and the C API has no way to declare one. Without `FILE_EXTENSION` the name
        // ends in a bare dot, which no image viewer opens.
        if path.ends_with('.') {
            let name = self.format.name();
            return Err(Error::invalid_input(format!(
                "{}: the file name '{path}' has no extension; with PARTITION_BY, add \
                 FILE_EXTENSION '{name}' to the COPY options to get data_0.{name}",
                self.format.label()
            )));
        }
        Ok(ChartFile {
            path: path.to_string(),
            rows: AtomicUsize::new(0),
            chart: Mutex::new(None),
        })
    }

    fn sink(
        &self,
        _options: &Options,
        batch: &mut ChartBatch,
        chunk: &CopyChunk<'_>,
    ) -> Result<()> {
        let column = chunk.column(0);
        if batch.rows == 0 && chunk.rows() > 0 {
            let bytes = if column.is_valid(0) {
                Some(column.bytes(0)?.to_vec())
            } else {
                None
            };
            *lock(&batch.first)? = Some(bytes);
        }
        batch.rows += chunk.rows();
        Ok(())
    }

    fn flush(&self, _options: &Options, file: &ChartFile, batch: &ChartBatch) -> Result<()> {
        let before = file.rows.fetch_add(batch.rows, Ordering::SeqCst);
        if before + batch.rows > 1 {
            return Err(Error::invalid_input(format!(
                "{}: a file holds one chart, and the query gave more than one row for '{}'; \
                 use PARTITION_BY to write one file per group, or one COPY per chart",
                self.format.label(),
                file.path
            )));
        }
        if let Some(chart) = lock(&batch.first)?.take() {
            *lock(&file.chart)? = Some(chart);
        }
        Ok(())
    }

    fn finalize(&self, options: &Options, file: &ChartFile) -> Result<()> {
        let label = self.format.label();
        let bytes = match lock(&file.chart)?.take() {
            None => {
                return Err(Error::invalid_input(format!(
                    "{label}: no chart to write to '{}'",
                    file.path
                )));
            }
            Some(None) => {
                return Err(Error::invalid_input(format!(
                    "{label}: the chart to write to '{}' is NULL",
                    file.path
                )));
            }
            Some(Some(bytes)) => bytes,
        };
        let chart = Root::decode(&bytes).map_err(|e| chart_error(&label, e))?;
        let image = self
            .format
            .render(&chart, options.width, options.height)
            .map_err(|e| chart_error(&label, e))?;
        write_file(&file.path, &image)
            .map_err(|e| Error::io(format!("{label}: cannot write '{}': {e}", file.path)))
    }
}

impl ChartCopy {
    /// The query must give exactly one column, of type `CHART`.
    fn check_columns(&self, b: &CopyBind<'_>) -> Result<()> {
        let label = self.format.label();
        let count = b.column_count()?;
        if count != 1 {
            let names = (0..count)
                .map(|i| b.column_name(i))
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            return Err(Error::binder(format!(
                "{label} takes exactly one column, a CHART, got {count} columns ({names})"
            )));
        }
        let ty = b.column_type(0)?;
        if ty.same_as(&self.chart) {
            return Ok(());
        }
        let text = ty.to_text();
        // Custom types over BLOB (SERIES, MESH, ...) report the BLOB id too; only the plain types
        // get the hint.
        let output = match (ty.id(), text.as_str()) {
            (TypeId::BLOB, "BLOB") => Some(("to_png()", "the BLOB")),
            (TypeId::VARCHAR, "VARCHAR") => Some(("to_svg()", "to_svg().encode()")),
            _ => None,
        };
        let hint = output.map_or_else(String::new, |(f, blob)| {
            format!(
                "; {label} renders the CHART itself, so pass the chart without {f} \
                 (or write {blob} with FORMAT blob)"
            )
        });
        Err(Error::binder(format!(
            "{label} takes a CHART column, got {} of type {text}{hint}",
            b.column_name(0)?
        )))
    }
}

fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| Error::internal("copy function: a thread panicked holding the file state"))
}

/// The path on the local file system, or `None` for a path DuckDB would hand to another file
/// system (`s3://`, `https://`, ...). DuckDB's local file system accepts a `file://` prefix.
fn local_path(path: &str) -> Option<&str> {
    match path.split_once("://") {
        None => Some(path),
        Some(("file", rest)) => Some(rest),
        Some(_) => None,
    }
}

/// Writes the whole file, and removes what was written if that fails part way, so that a failed
/// `COPY` leaves no partial image behind.
fn write_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = local_path(path).unwrap_or(path);
    std::fs::write(path, bytes).inspect_err(|_| {
        let _ = std::fs::remove_file(path);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths() {
        assert_eq!(local_path("s3://bucket/chart.png"), None);
        assert_eq!(local_path("https://example.com/chart.png"), None);
        assert_eq!(local_path("file:///tmp/chart.png"), Some("/tmp/chart.png"));
        assert_eq!(local_path("/tmp/chart.png"), Some("/tmp/chart.png"));
        assert_eq!(local_path("charts/city=Oslo"), Some("charts/city=Oslo"));
    }

    #[test]
    fn a_failed_write_leaves_no_file() {
        let dir = std::env::temp_dir().join(format!("plume-copy-{}", std::process::id()));
        let missing = dir.join("no-such-dir").join("chart.png");
        assert!(write_file(missing.to_str().unwrap(), b"png").is_err());
        assert!(!missing.exists());
    }
}
