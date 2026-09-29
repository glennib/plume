//! The plume custom types and their casts.
//!
//! Every value is a `BLOB` in `plume-chart`'s encoding ([`plume_chart::Value`]): a 4-byte
//! magic naming the type, the format version, then the postcard-encoded spec.

use std::fmt;

use plume_chart::{Font, Mesh, Root, Series, SeriesLabels, Value};

use crate::capi::{Error, Extension, LogicalType, Result, TypeId};

/// The kinds of plume values, one per custom SQL type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Chart,
    Mesh,
    SeriesLabels,
    Series,
    Font,
}

impl ValueKind {
    pub const ALL: [ValueKind; 5] = [
        ValueKind::Chart,
        ValueKind::Mesh,
        ValueKind::SeriesLabels,
        ValueKind::Series,
        ValueKind::Font,
    ];

    /// The SQL type name.
    pub fn sql_name(self) -> &'static str {
        match self {
            ValueKind::Chart => Root::TYPE_NAME,
            ValueKind::Mesh => Mesh::TYPE_NAME,
            ValueKind::SeriesLabels => SeriesLabels::TYPE_NAME,
            ValueKind::Series => Series::TYPE_NAME,
            ValueKind::Font => Font::TYPE_NAME,
        }
    }

    /// Decodes `bytes` as a value of this kind and returns its one-line summary, which is what
    /// `value::VARCHAR` returns and what the DuckDB CLI shows for the value in every output mode.
    pub fn summary(self, bytes: &[u8]) -> plume_chart::Result<String> {
        Ok(match self {
            ValueKind::Chart => Root::decode(bytes)?.summary(),
            ValueKind::Mesh => Mesh::decode(bytes)?.summary(),
            ValueKind::SeriesLabels => SeriesLabels::decode(bytes)?.summary(),
            ValueKind::Series => Series::decode(bytes)?.summary(),
            ValueKind::Font => Font::decode(bytes)?.summary(),
        })
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.sql_name())
    }
}

/// Logical types used across the function registrations: the built-in ones plume takes and
/// returns, and the custom types registered here.
pub struct Types {
    pub any: LogicalType,
    pub varchar: LogicalType,
    pub blob: LogicalType,
    pub boolean: LogicalType,
    pub integer: LogicalType,
    pub bigint: LogicalType,
    pub double: LogicalType,
    pub date: LogicalType,
    pub timestamp: LogicalType,
    pub timestamp_tz: LogicalType,
    pub chart: LogicalType,
    pub mesh: LogicalType,
    pub series_labels: LogicalType,
    pub series: LogicalType,
    /// `SERIES[]`, what a series aggregate called with `key :=` returns.
    pub series_list: LogicalType,
    pub font: LogicalType,
    /// `CHART[]`, what `split_evenly` takes.
    pub chart_list: LogicalType,
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
/// to `VARCHAR` (the one-line summary) and to and from `BLOB` (the raw bytes; `BLOB → <type>`
/// checks that the bytes decode as a value of the type).
///
/// The C API registers no casts between a custom type and its base type, so without these a
/// `CHART` could neither be displayed nor written with `COPY ... (FORMAT blob)` after `::BLOB`.
pub fn register(ext: &Extension<'_>) -> Result<Types> {
    let ctx = ext.context();
    let blob = ctx.type_from_id(TypeId::BLOB)?;
    let varchar = ctx.type_from_id(TypeId::VARCHAR)?;

    let mut custom = Vec::with_capacity(ValueKind::ALL.len());
    for kind in ValueKind::ALL {
        let ty = ext.register_type(kind.sql_name(), &blob)?;

        ext.register_bytes_cast(&ty, &varchar, move |input, row| {
            let text = kind
                .summary(input.bytes(row)?)
                .map_err(|e| Error::conversion(e.to_string()))?;
            Ok(Some(text.into_bytes()))
        })?;
        ext.register_bytes_cast(&ty, &blob, |input, row| {
            Ok(Some(input.bytes(row)?.to_vec()))
        })?;
        ext.register_bytes_cast(&blob, &ty, move |input, row| {
            let bytes = input.bytes(row)?;
            kind.summary(bytes)
                .map_err(|e| Error::conversion(e.to_string()))?;
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
        date: ctx.type_from_id(TypeId::DATE)?,
        timestamp: ctx.type_from_id(TypeId::TIMESTAMP)?,
        timestamp_tz: ctx.type_from_id(TypeId::TIMESTAMP_TZ)?,
        series_list: ctx.type_from_text("SERIES[]")?,
        chart_list: ctx.type_from_text("CHART[]")?,
        varchar,
        blob,
        chart,
        mesh,
        series_labels,
        series,
        font,
    })
}
