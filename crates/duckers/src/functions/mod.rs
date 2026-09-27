//! The SQL functions duckers registers.

mod args;
mod chart;
mod internal;
mod series;

use crate::capi::{Extension, Result, ScalarFunction};
use crate::types::Types;

/// The extension version, as written into the metadata footer (`v` + the crate version).
pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

pub fn register(ext: &Extension<'_>, types: &Types) -> Result<()> {
    ext.register_scalar(ScalarFunction::map_rows(
        "duckers_version",
        &types.varchar,
        |_, _| Ok(Some(VERSION)),
    ))?;
    series::register(ext, types)?;
    chart::register(ext, types)?;
    internal::register(ext, types)
}
