//! The chart core of duckers: the values behind the SQL types `CHART`, `SERIES`, `MESH`,
//! `SERIES_LABELS` and `FONT`, the methods on them, the state of the series aggregates, and the
//! plotters renderer. Nothing here depends on DuckDB; the extension converts DuckDB vectors to
//! calls on this crate.
//!
//! - [`spec`]: the value types, plain data.
//! - [`envelope`]: their BLOB encoding ([`Value::encode`], [`Value::decode`]).
//! - [`methods`](crate::methods): the SQL methods (`caption`, `draw_series`, `style`, ...) as
//!   functions on the value types.
//! - [`accumulate`]: the series aggregates' state (`line_series`, `histogram_vertical`,
//!   `candle_stick`, ...; [`SeriesAggregate`] lists them).
//! - [`color`]: colour strings and `mix`.
//! - [`layout`]: the grid, titled and pie roots of a `CHART` ([`Root`]) and the `pie`
//!   aggregate's state.
//! - [`render`]: `to_svg`, `to_png`, `to_rgb`.
//! - Summaries for the `VARCHAR` casts are the `summary()` methods of the value types.
//!
//! Every fallible call returns [`Error`], whose message is meant for the SQL user.

pub mod accumulate;
pub mod color;
pub mod envelope;
mod error;
mod full_palette;
pub mod label_format;
pub mod layout;
pub mod methods;
pub mod render;
pub mod spec;
mod summary;

pub use accumulate::{Accumulator, Key, SeriesAggregate, SeriesBinding, SortKey, SqlType, XValue};
pub use color::{Color, mix};
pub use envelope::{FORMAT_VERSION, Value};
pub use error::{Error, ErrorKind, Result};
pub use layout::PieAccumulator;
pub use methods::RangeValue;
pub use render::{
    DEFAULT_HEIGHT, DEFAULT_WIDTH, MAX_SIDE, image_size, render_rgb, to_png, to_rgb, to_svg,
};
pub use spec::{Chart, Column, Font, Mesh, Root, Series, SeriesLabels};
