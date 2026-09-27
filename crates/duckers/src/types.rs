//! The duckers custom types and their casts.

use crate::capi::{Error, Extension, LogicalType, Result, TypeId};
use crate::envelope::{self, Summarizer, ValueKind};

/// Logical types used across the function registrations: the built-in ones duckers takes and
/// returns, and the custom types registered here.
pub struct Types {
    pub any: LogicalType,
    pub varchar: LogicalType,
    pub blob: LogicalType,
    pub boolean: LogicalType,
    pub integer: LogicalType,
    pub bigint: LogicalType,
    pub double: LogicalType,
    pub chart: LogicalType,
    pub mesh: LogicalType,
    pub series_labels: LogicalType,
    pub series: LogicalType,
    pub font: LogicalType,
}

impl Types {
    /// The custom type of a value kind.
    pub fn of(&self, kind: ValueKind) -> &LogicalType {
        match kind {
            ValueKind::Chart => &self.chart,
            ValueKind::Mesh => &self.mesh,
            ValueKind::SeriesLabels => &self.series_labels,
            ValueKind::Series => &self.series,
            ValueKind::Font => &self.font,
        }
    }
}

/// Registers `CHART`, `MESH`, `SERIES_LABELS`, `SERIES` and `FONT` over `BLOB`, each with casts
/// to `VARCHAR` (the one-line summary from `summarize`) and to and from `BLOB` (the raw bytes;
/// `BLOB → <type>` checks the envelope).
///
/// The C API registers no casts between a custom type and its base type, so without these a
/// `CHART` could neither be displayed nor written with `COPY ... (FORMAT blob)` after `::BLOB`.
pub fn register(ext: &Extension<'_>, summarize: Summarizer) -> Result<Types> {
    let ctx = ext.context();
    let blob = ctx.type_from_id(TypeId::BLOB)?;
    let varchar = ctx.type_from_id(TypeId::VARCHAR)?;

    let mut custom = Vec::with_capacity(ValueKind::ALL.len());
    for kind in ValueKind::ALL {
        let ty = ext.register_type(kind.sql_name(), &blob)?;

        ext.register_bytes_cast(&ty, &varchar, move |input, row| {
            let payload = envelope::decode(kind, input.bytes(row)?)
                .map_err(|e| Error::conversion(e.to_string()))?;
            let text =
                summarize(kind, payload).map_err(|e| Error::conversion(format!("{kind}: {e}")))?;
            Ok(Some(text.into_bytes()))
        })?;
        ext.register_bytes_cast(&ty, &blob, |input, row| {
            Ok(Some(input.bytes(row)?.to_vec()))
        })?;
        ext.register_bytes_cast(&blob, &ty, move |input, row| {
            let bytes = input.bytes(row)?;
            envelope::decode(kind, bytes).map_err(|e| Error::conversion(e.to_string()))?;
            Ok(Some(bytes.to_vec()))
        })?;
        custom.push(ty);
    }
    let [chart, mesh, series_labels, series, font]: [LogicalType; 5] = custom
        .try_into()
        .map_err(|_| Error::internal("custom type count"))?;

    Ok(Types {
        any: ctx.type_from_id(TypeId::ANY)?,
        boolean: ctx.type_from_id(TypeId::BOOLEAN)?,
        integer: ctx.type_from_id(TypeId::INTEGER)?,
        bigint: ctx.type_from_id(TypeId::BIGINT)?,
        double: ctx.type_from_id(TypeId::DOUBLE)?,
        varchar,
        blob,
        chart,
        mesh,
        series_labels,
        series,
        font,
    })
}
