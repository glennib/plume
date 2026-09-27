//! Internal functions for tests, prefixed `__duckers_`. They are not part of the user API and may
//! change or disappear in any release.

use duckers_chart::{Font, Mesh, Root, Series, SeriesLabels, Value};

use crate::capi::{
    Aggregate, AggregateFunction, AggregateInput, Bind, Error, Extension, LogicalType,
    OutputVector, Result, ScalarFunction, TypeId,
};
use crate::types::Types;

/// `__duckers_debug(value)`: the Rust `Debug` text of a decoded value, so tests can see the
/// numbers inside a `SERIES` or `CHART`.
fn debug<T: Value + std::fmt::Debug>(ty: &LogicalType, types: &Types) -> ScalarFunction {
    ScalarFunction::map_rows("__duckers_debug", &types.varchar, |input, row| {
        let value =
            T::decode(input.arg(0).bytes(row)?).map_err(|e| Error::invalid_input(e.to_string()))?;
        Ok(Some(format!("{value:?}")))
    })
    .param("value", ty)
}

pub fn register(ext: &Extension<'_>, types: &Types) -> Result<()> {
    ext.register_scalar(debug::<Root>(&types.chart, types))?;
    ext.register_scalar(debug::<Series>(&types.series, types))?;
    ext.register_scalar(debug::<Mesh>(&types.mesh, types))?;
    ext.register_scalar(debug::<SeriesLabels>(&types.series_labels, types))?;
    ext.register_scalar(debug::<Font>(&types.font, types))?;

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
