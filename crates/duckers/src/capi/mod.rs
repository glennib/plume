//! A small safe layer over the DuckDB v2 C API (through `duckers-sys`): registration of scalar
//! functions, aggregates, custom types and casts, plus reading and writing vectors.
//!
//! Callbacks are Rust closures or [`Aggregate`] impls; the `extern "C"` trampolines here catch
//! errors and panics and report them through DuckDB's error slot.

mod aggregate;
mod cast;
mod error;
mod extension;
mod function;
mod scalar;
mod types;
mod vector;

pub use aggregate::{Aggregate, AggregateFunction, AggregateInput};
pub use error::{Error, Result};
pub use extension::{Extension, entrypoint};
pub use function::{Bind, BindData, Param};
pub use scalar::{ScalarFunction, ScalarInput, WriteCell};
pub use types::{Context, LogicalType, TypeId, Value};
pub use vector::{InputVector, OutputVector};
