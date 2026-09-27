//! M0 spike 1: the C API features duckers relies on, exercised from Rust.

use crate::capi::{
    Aggregate, AggregateFunction, AggregateInput, Bind, Error, Extension, LogicalType,
    OutputVector, Result, ScalarFunction, TypeId,
};

pub fn load(ext: &Extension<'_>) -> Result<()> {
    let ctx = ext.context();
    let varchar = ctx.type_from_id(TypeId::VARCHAR)?;
    let bigint = ctx.type_from_id(TypeId::BIGINT)?;
    let any = ctx.type_from_id(TypeId::ANY)?;
    let blob = ctx.type_from_id(TypeId::BLOB)?;

    // Scalar with named parameters.
    ext.register_scalar(
        ScalarFunction::map_rows("spike_join", &varchar, |input, row| {
            let x = input.arg(0).i64(row)?;
            let sep = input.arg(1).str(row)?;
            let times = input.arg(2).i64(row)?;
            let argc_bind = input.bind_data::<usize>().copied().unwrap_or(0);
            let body = std::iter::repeat_n(x.to_string(), times.max(0) as usize)
                .collect::<Vec<_>>()
                .join(sep);
            Ok(Some(format!(
                "{body} (exec args {}, bind args {argc_bind})",
                input.arg_count()
            )))
        })
        .param("x", &bigint)
        .named("sep", &varchar, ctx.varchar(",")?)
        .named("times", &bigint, ctx.bigint(2)?)
        .bind(|b| Ok(Some(Box::new(b.arg_count())))),
    )?;

    // Order-independent aggregate with ANY and named parameters.
    ext.register_aggregate(
        AggregateFunction::<SpikeAgg>::new("spike_agg", &varchar)
            .param("x", &any)
            .param("y", &any)
            .named("label", &varchar, ctx.varchar("none")?)
            .named("key", &any, ctx.null(&varchar)?),
    )?;

    // Custom type over BLOB with casts.
    let chart = ext.register_type("SPIKE_CHART", &blob)?;
    ext.register_bytes_cast(&chart, &varchar, |input, row| {
        let n = input.bytes(row)?.len();
        Ok(Some(format!("SPIKE_CHART({n} bytes)").into_bytes()))
    })?;
    ext.register_bytes_cast(&chart, &blob, |input, row| {
        Ok(Some(input.bytes(row)?.to_vec()))
    })?;
    ext.register_bytes_cast(&blob, &chart, |input, row| {
        let b = input.bytes(row)?;
        if b.starts_with(b"SPK") {
            Ok(Some(b.to_vec()))
        } else {
            Err(Error::conversion("not a SPIKE_CHART value"))
        }
    })?;
    ext.register_scalar(
        ScalarFunction::map_rows("spike_chart", &chart, |input, row| {
            let mut v = b"SPK".to_vec();
            v.extend_from_slice(input.arg(0).bytes(row)?);
            Ok(Some(v))
        })
        .param("s", &varchar),
    )?;
    Ok(())
}

struct SpikeAggBind {
    label: Option<String>,
    arg_types: Vec<String>,
}

#[derive(Default)]
struct SpikeAgg {
    rows: u64,
    skipped: u64,
    sum_x: f64,
    sum_y: f64,
    label: Option<String>,
}

impl Aggregate for SpikeAgg {
    type BindData = SpikeAggBind;

    fn bind(b: &mut Bind<'_>) -> Result<SpikeAggBind> {
        let numeric = |i: usize, name: &str| -> Result<LogicalType> {
            let ty = b.arg_type(i)?;
            if ty.id().is_plain_numeric() {
                Ok(ty)
            } else {
                Err(Error::binder(format!(
                    "spike_agg: {name} must be numeric, got {}",
                    ty.to_text()
                )))
            }
        };
        numeric(0, "x")?;
        numeric(1, "y")?;
        let arg_types = (0..b.arg_count())
            .map(|i| b.arg_type(i).map(|t| t.to_text()))
            .collect::<Result<Vec<_>>>()?;
        // A constant label folds at bind time; a column label arrives per row in update.
        let label = b
            .arg_value(2)
            .ok()
            .and_then(|v| (!v.is_null()).then(|| v.as_str().ok()).flatten());
        Ok(SpikeAggBind { label, arg_types })
    }

    fn init(bind: &SpikeAggBind) -> Self {
        Self {
            label: bind.label.clone(),
            ..Self::default()
        }
    }

    fn update(
        &mut self,
        _bind: &SpikeAggBind,
        input: &AggregateInput<'_>,
        row: usize,
    ) -> Result<()> {
        if !(input.arg(0).is_valid(row) && input.arg(1).is_valid(row)) {
            self.skipped += 1;
            return Ok(());
        }
        self.rows += 1;
        self.sum_x += input.arg(0).f64(row)?;
        self.sum_y += input.arg(1).f64(row)?;
        if self.label.is_none() && input.arg(2).is_valid(row) {
            self.label = Some(input.arg(2).str(row)?.to_string());
        }
        Ok(())
    }

    fn combine(&mut self, _bind: &SpikeAggBind, other: &Self) -> Result<()> {
        self.rows += other.rows;
        self.skipped += other.skipped;
        self.sum_x += other.sum_x;
        self.sum_y += other.sum_y;
        if self.label.is_none() {
            self.label.clone_from(&other.label);
        }
        Ok(())
    }

    fn finalize(
        &mut self,
        bind: &SpikeAggBind,
        out: &mut OutputVector<'_>,
        row: usize,
    ) -> Result<()> {
        out.set_str(
            row,
            &format!(
                "{}: rows={} skipped={} sum_x={} sum_y={} args=({})",
                self.label.as_deref().unwrap_or("?"),
                self.rows,
                self.skipped,
                self.sum_x,
                self.sum_y,
                bind.arg_types.join(", ")
            ),
        )
    }
}
