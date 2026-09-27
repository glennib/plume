//! Internal functions for tests, prefixed `__duckers_`. They are not part of the user API and may
//! change or disappear in any release.

use crate::capi::{
    Aggregate, AggregateFunction, AggregateInput, Bind, Error, Extension, OutputVector, Result,
    ScalarFunction, TypeId,
};
use crate::envelope::{self, ValueKind};
use crate::types::Types;

pub fn register(ext: &Extension<'_>, types: &Types) -> Result<()> {
    // `__duckers_envelope('CHART', payload)` wraps raw bytes in a value envelope and returns them
    // as a BLOB, so tests can build values of every type before the constructors exist:
    // `__duckers_envelope('CHART', 'x'::BLOB)::CHART`.
    ext.register_scalar(
        ScalarFunction::map_rows("__duckers_envelope", &types.blob, |input, row| {
            let name = input.arg(0).str(row)?;
            let kind = ValueKind::from_sql_name(name)
                .ok_or_else(|| Error::invalid_input(format!("unknown duckers type {name}")))?;
            Ok(Some(envelope::encode(kind, input.arg(1).bytes(row)?)))
        })
        .param("kind", &types.varchar)
        .param("payload", &types.blob),
    )?;

    // `__duckers_agg_probe(x, y, label := ...)` exercises the aggregate layer the series
    // aggregates build on: ANY parameters checked in bind, a named parameter with a default, NULL
    // rows, grouping and combine.
    ext.register_aggregate(
        AggregateFunction::<AggProbe>::new("__duckers_agg_probe", &types.varchar)
            .param("x", &types.any)
            .param("y", &types.any)
            .named("label", &types.varchar, ext.context().varchar("probe")?),
    )
}

#[derive(Default)]
struct AggProbe {
    rows: u64,
    skipped: u64,
    sum_x: f64,
    sum_y: f64,
}

impl Aggregate for AggProbe {
    type BindData = String;

    fn bind(b: &mut Bind<'_>) -> Result<String> {
        const NUMERIC: &[TypeId] = &[
            TypeId::TINYINT,
            TypeId::SMALLINT,
            TypeId::INTEGER,
            TypeId::BIGINT,
            TypeId::UTINYINT,
            TypeId::USMALLINT,
            TypeId::UINTEGER,
            TypeId::UBIGINT,
            TypeId::FLOAT,
            TypeId::DOUBLE,
        ];
        b.expect_arg(0, "x", NUMERIC, "a number")?;
        b.expect_arg(1, "y", NUMERIC, "a number")?;
        b.constant_arg(2, "label")?.as_str()
    }

    fn init(_: &String) -> Self {
        Self::default()
    }

    fn update(&mut self, _: &String, input: &AggregateInput<'_>, row: usize) -> Result<()> {
        if !input.all_valid(row) {
            self.skipped += 1;
            return Ok(());
        }
        self.rows += 1;
        self.sum_x += input.arg(0).f64(row)?;
        self.sum_y += input.arg(1).f64(row)?;
        Ok(())
    }

    fn combine(&mut self, _: &String, other: &Self) -> Result<()> {
        self.rows += other.rows;
        self.skipped += other.skipped;
        self.sum_x += other.sum_x;
        self.sum_y += other.sum_y;
        Ok(())
    }

    fn finalize(&mut self, label: &String, out: &mut OutputVector<'_>, row: usize) -> Result<()> {
        out.set_str(
            row,
            &format!(
                "{label}: rows={} skipped={} sum_x={} sum_y={}",
                self.rows, self.skipped, self.sum_x, self.sum_y
            ),
        )
    }
}
