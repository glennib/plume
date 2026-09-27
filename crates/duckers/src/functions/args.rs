//! Reading SQL arguments into `duckers-chart` values, and reporting its errors as SQL errors.

use duckers_chart::{Chart, ErrorKind, RangeValue, Root, SortKey, SqlType, Value, XValue};

use crate::capi::{
    Bind, Error, InputVector, LogicalType, Result, ScalarFunction, ScalarInput, TypeId, WriteCell,
};

/// A `duckers-chart` error as the SQL error of function `name`. The message names the function
/// unless it already starts with its name.
pub fn chart_error(name: &str, e: duckers_chart::Error) -> Error {
    let message = e.message();
    let message = if message.starts_with(name) {
        message.to_string()
    } else {
        format!("{name}: {message}")
    };
    match e.kind() {
        ErrorKind::Decode | ErrorKind::Invalid => Error::invalid_input(message),
        ErrorKind::Render => Error::internal(message),
    }
}

/// The arguments of one row of a scalar function call, all non-NULL.
pub struct Args<'a, 'b> {
    input: &'a ScalarInput<'b>,
    row: usize,
    name: &'static str,
}

impl Args<'_, '_> {
    pub fn row(&self) -> usize {
        self.row
    }

    pub fn arg(&self, index: usize) -> &InputVector<'_> {
        self.input.arg(index)
    }

    /// Converts a `duckers-chart` result, naming this function in the error.
    pub fn check<T>(&self, result: duckers_chart::Result<T>) -> Result<T> {
        result.map_err(|e| chart_error(self.name, e))
    }

    /// Argument `index` decoded as a duckers value.
    pub fn value<T: Value>(&self, index: usize) -> Result<T> {
        self.check(T::decode(self.arg(index).bytes(self.row)?))
    }

    /// Argument `index`, a `CHART`, as the cartesian chart a `ChartBuilder` or `ChartContext`
    /// method works on: an error naming this function for a grid, titled or pie root.
    pub fn chart(&self, index: usize) -> Result<Chart> {
        let root: Root = self.value(index)?;
        self.check(root.cartesian(self.name))
    }

    pub fn str(&self, index: usize) -> Result<&str> {
        self.arg(index).str(self.row)
    }

    pub fn i64(&self, index: usize) -> Result<i64> {
        self.arg(index).i64(self.row)
    }

    pub fn f64(&self, index: usize) -> Result<f64> {
        self.arg(index).f64(self.row)
    }

    /// Argument `index` as an axis range bound, by its type: see [`check_range_bound`].
    pub fn range_bound(&self, index: usize) -> Result<RangeValue> {
        let v = self.arg(index);
        let id = v.type_id();
        if id == TypeId::DATE {
            // SAFETY: DATE is stored as i32 days.
            Ok(RangeValue::Date(unsafe { v.get::<i32>(self.row) }))
        } else if id.is_timestamp() {
            Ok(RangeValue::Timestamp(timestamp_micros(v, self.row)))
        } else {
            Ok(RangeValue::Number(v.f64(self.row)?))
        }
    }
}

/// A scalar function computed row by row from non-NULL arguments (NULL in, NULL out).
pub fn scalar<R: WriteCell>(
    name: &'static str,
    returns: &LogicalType,
    f: impl Fn(&Args<'_, '_>) -> Result<R> + Send + Sync + 'static,
) -> ScalarFunction {
    ScalarFunction::map_rows(name, returns, move |input, row| {
        f(&Args { input, row, name }).map(Some)
    })
}

/// Checks that argument `index` (parameter `param`) can bound an axis range: any numeric type,
/// `DATE`, `TIMESTAMP`, `TIMESTAMPTZ` or a `TIMESTAMP_*` precision.
pub fn check_range_bound(b: &Bind<'_>, index: usize, param: &str) -> Result<()> {
    let ty = b.arg_type(index)?;
    let id = ty.id();
    if id.is_numeric() || id == TypeId::DATE || id.is_timestamp() {
        Ok(())
    } else {
        Err(Error::binder(format!(
            "{}: {param} must be a number, DATE or TIMESTAMP, got {}",
            b.function_name(),
            ty.to_text()
        )))
    }
}

/// A `TIMESTAMP*` value as microseconds since the epoch, whatever the precision.
fn timestamp_micros(v: &InputVector<'_>, row: usize) -> i64 {
    // SAFETY: every timestamp type is stored as i64 in its own unit.
    let raw = unsafe { v.get::<i64>(row) };
    match v.type_id() {
        TypeId::TIMESTAMP_S => raw.saturating_mul(1_000_000),
        TypeId::TIMESTAMP_MS => raw.saturating_mul(1_000),
        TypeId::TIMESTAMP_NS | TypeId::TIMESTAMP_TZ_NS => raw.div_euclid(1_000),
        _ => raw,
    }
}

/// How a series aggregate reads its x (or bucket) argument, decided at bind time.
#[derive(Clone, Copy, Debug)]
pub enum XReader {
    Integer,
    Float,
    Date,
    Timestamp,
    Varchar,
}

impl XReader {
    /// The reader and `duckers-chart` type family for an argument type, if it can be an x value.
    pub fn for_type(id: TypeId) -> Option<(XReader, SqlType)> {
        Some(if id.is_integer() {
            (XReader::Integer, SqlType::Integer)
        } else if id.is_numeric() {
            (XReader::Float, SqlType::Float)
        } else if id == TypeId::DATE {
            (XReader::Date, SqlType::Date)
        } else if id.is_timestamp() {
            (XReader::Timestamp, SqlType::Timestamp)
        } else if id == TypeId::VARCHAR {
            (XReader::Varchar, SqlType::Varchar)
        } else {
            return None;
        })
    }

    /// The x value at `row`, or `None` for a value the aggregate skips (`±infinity` dates and
    /// timestamps, which no axis can place).
    pub fn read(self, v: &InputVector<'_>, row: usize) -> Result<Option<XValue>> {
        Ok(Some(match self {
            XReader::Integer => match i64::try_from(v.i128(row)?) {
                Ok(i) => XValue::Integer(i),
                Err(_) => XValue::Number(v.f64(row)?),
            },
            XReader::Float => XValue::Number(v.f64(row)?),
            XReader::Date => {
                // SAFETY: DATE is stored as i32 days.
                let days = unsafe { v.get::<i32>(row) };
                if days == i32::MAX || days == -i32::MAX {
                    return Ok(None);
                }
                XValue::Date(days)
            }
            XReader::Timestamp => {
                // SAFETY: every timestamp type is stored as i64.
                let raw = unsafe { v.get::<i64>(row) };
                if raw == i64::MAX || raw == -i64::MAX {
                    return Ok(None);
                }
                XValue::Timestamp(timestamp_micros(v, row))
            }
            XReader::Varchar => XValue::Category(v.str(row)?.to_string()),
        }))
    }
}

/// The value at `row` of a `key` or `order_by` argument of any type, in DuckDB's sort order for
/// the common types and lists of them. Other types (`UUID`, `ENUM`, `STRUCT`, ...) sort by their
/// text, as `::VARCHAR` renders it.
pub fn sort_key(v: &InputVector<'_>, row: usize) -> Result<SortKey> {
    if !v.is_valid(row) {
        return Ok(SortKey::Null);
    }
    let id = v.type_id();
    // SAFETY: each arm reads the physical type the type id implies.
    Ok(unsafe {
        match id {
            TypeId::BOOLEAN => SortKey::Bool(v.get::<u8>(row) != 0),
            TypeId::FLOAT | TypeId::DOUBLE => SortKey::Float(v.f64(row)?),
            // One column has one scale, so the unscaled integers sort like the decimals.
            TypeId::DECIMAL => SortKey::Int(v.decimal_unscaled(row)?),
            TypeId::DATE => SortKey::Int(i128::from(v.get::<i32>(row))),
            TypeId::TIME | TypeId::TIME_NS => SortKey::Int(i128::from(v.get::<i64>(row))),
            TypeId::VARCHAR => SortKey::Text(v.str(row)?.to_string()),
            TypeId::BLOB => SortKey::Bytes(v.bytes(row)?.to_vec()),
            TypeId::LIST => {
                let elements = v.list_child()?;
                let items = v.list_entry(row)?.map(|i| sort_key(&elements, i));
                SortKey::List(items.collect::<Result<_>>()?)
            }
            // DuckDB compares intervals normalised to 30-day months.
            TypeId::INTERVAL => {
                let i = v.get::<duckers_sys::duckdb_v2_interval_t>(row);
                let day = 86_400_000_000i128;
                SortKey::Int(
                    (i128::from(i.months) * 30 + i128::from(i.days)) * day + i128::from(i.micros),
                )
            }
            _ if id.is_timestamp() => SortKey::Int(i128::from(v.get::<i64>(row))),
            _ if id.is_integer() && id != TypeId::UHUGEINT => SortKey::Int(v.i128(row)?),
            _ => SortKey::Text(v.value(row)?.to_display()),
        }
    })
}

/// The label of a `key` value: `key::VARCHAR`, or `None` for a NULL key.
pub fn key_label(v: &InputVector<'_>, row: usize) -> Result<Option<String>> {
    if !v.is_valid(row) {
        return Ok(None);
    }
    let id = v.type_id();
    Ok(Some(if id == TypeId::VARCHAR {
        v.str(row)?.to_string()
    } else if id.is_plain_integer() {
        v.i128(row)?.to_string()
    } else {
        v.value(row)?.to_display()
    }))
}
