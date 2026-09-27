//! Logical types, values and the borrowed client context that creates them.

use std::ffi::c_char;
use std::fmt;
use std::marker::PhantomData;
use std::ptr;

use duckers_sys::{self as sys, ffi};

use super::error::{Result, check};

/// A borrowed client context, valid for the callback (or entrypoint) that received it.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    handle: sys::duckdb_v2_context_handle,
    _scope: PhantomData<&'a ()>,
}

impl Context<'_> {
    /// # Safety
    ///
    /// `handle` must be a live context for the lifetime the caller picks.
    pub(crate) unsafe fn from_raw(handle: sys::duckdb_v2_context_handle) -> Self {
        Self {
            handle,
            _scope: PhantomData,
        }
    }

    pub fn raw(&self) -> sys::duckdb_v2_context_handle {
        self.handle
    }

    /// A type by id, for the types that take no parameters (`BIGINT`, `VARCHAR`, `ANY`, ...).
    pub fn type_from_id(&self, id: TypeId) -> Result<LogicalType> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_context_create_type_from_id(
                self.handle,
                id.0,
                ptr::null(),
                ptr::null(),
                0,
                &mut out,
                err
            ))
        })?;
        Ok(LogicalType(out))
    }

    /// A type parsed from SQL type text, e.g. `"CHART"` or `"DOUBLE[]"`.
    pub fn type_from_text(&self, text: &str) -> Result<LogicalType> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_context_create_type_from_text(
                self.handle,
                sys::str_view(text.as_bytes()),
                &mut out,
                err
            ))
        })?;
        Ok(LogicalType(out))
    }

    pub fn varchar(&self, s: &str) -> Result<Value> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_value_create_varchar_with_context(
                self.handle,
                sys::str_view(s.as_bytes()),
                &mut out,
                err
            ))
        })?;
        Ok(Value(out))
    }

    pub fn bigint(&self, v: i64) -> Result<Value> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_value_create_bigint_with_context(
                self.handle,
                v,
                &mut out,
                err
            ))
        })?;
        Ok(Value(out))
    }

    pub fn double(&self, v: f64) -> Result<Value> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_value_create_double_with_context(
                self.handle,
                v,
                &mut out,
                err
            ))
        })?;
        Ok(Value(out))
    }

    pub fn boolean(&self, v: bool) -> Result<Value> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_value_create_bool_with_context(
                self.handle,
                v,
                &mut out,
                err
            ))
        })?;
        Ok(Value(out))
    }

    /// A typed NULL.
    pub fn null(&self, ty: &LogicalType) -> Result<Value> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_value_create_null_with_context(
                self.handle,
                ty.0,
                &mut out,
                err
            ))
        })?;
        Ok(Value(out))
    }
}

/// A logical type id (`DUCKDB_V2_LOGICAL_TYPE_ID_*`). Custom types report their base type's id.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub sys::DUCKDB_V2_LOGICAL_TYPE_ID);

impl TypeId {
    pub const INVALID: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_INVALID);
    pub const SQLNULL: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_SQLNULL);
    pub const ANY: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_ANY);
    pub const BOOLEAN: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_BOOLEAN);
    pub const TINYINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TINYINT);
    pub const SMALLINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_SMALLINT);
    pub const INTEGER: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_INTEGER);
    pub const BIGINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_BIGINT);
    pub const HUGEINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_HUGEINT);
    pub const UTINYINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_UTINYINT);
    pub const USMALLINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_USMALLINT);
    pub const UINTEGER: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_UINTEGER);
    pub const UBIGINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_UBIGINT);
    pub const UHUGEINT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_UHUGEINT);
    pub const FLOAT: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_FLOAT);
    pub const DOUBLE: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_DOUBLE);
    pub const DECIMAL: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_DECIMAL);
    pub const DATE: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_DATE);
    pub const TIME: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIME);
    pub const TIME_NS: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIME_NS);
    pub const TIMESTAMP_S: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIMESTAMP_SEC);
    pub const TIMESTAMP_MS: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIMESTAMP_MS);
    pub const TIMESTAMP: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIMESTAMP);
    pub const TIMESTAMP_NS: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIMESTAMP_NS);
    pub const TIMESTAMP_TZ: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIMESTAMP_TZ);
    pub const TIMESTAMP_TZ_NS: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_TIMESTAMP_TZ_NS);
    pub const VARCHAR: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_VARCHAR);
    pub const BLOB: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_BLOB);
    pub const INTERVAL: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_INTERVAL);
    pub const LIST: Self = Self(sys::DUCKDB_V2_LOGICAL_TYPE_ID_LIST);

    /// Whether values of this type read as a number through
    /// [`InputVector::f64`](super::vector::InputVector::f64): every integer width, `FLOAT`,
    /// `DOUBLE` and `DECIMAL`.
    pub fn is_numeric(self) -> bool {
        self.is_integer() || matches!(self, Self::FLOAT | Self::DOUBLE | Self::DECIMAL)
    }

    /// Whether this is an integer type of any width, signed or not, `HUGEINT` included.
    pub fn is_integer(self) -> bool {
        self.is_plain_integer() || matches!(self, Self::HUGEINT | Self::UHUGEINT)
    }

    /// Whether this is an integer type that fits `i64` or `u64` (`TINYINT` .. `UBIGINT`).
    pub fn is_plain_integer(self) -> bool {
        matches!(
            self,
            Self::TINYINT
                | Self::SMALLINT
                | Self::INTEGER
                | Self::BIGINT
                | Self::UTINYINT
                | Self::USMALLINT
                | Self::UINTEGER
                | Self::UBIGINT
        )
    }

    /// Whether this is `TIMESTAMP`, `TIMESTAMPTZ` or one of the `TIMESTAMP_*` precisions.
    pub fn is_timestamp(self) -> bool {
        matches!(
            self,
            Self::TIMESTAMP
                | Self::TIMESTAMP_S
                | Self::TIMESTAMP_MS
                | Self::TIMESTAMP_NS
                | Self::TIMESTAMP_TZ
                | Self::TIMESTAMP_TZ_NS
        )
    }

    /// Whether values of this type read as a plain number through
    /// [`InputVector::f64`](super::vector::InputVector::f64) with a single primitive load.
    pub fn is_plain_numeric(self) -> bool {
        matches!(
            self,
            Self::TINYINT
                | Self::SMALLINT
                | Self::INTEGER
                | Self::BIGINT
                | Self::UTINYINT
                | Self::USMALLINT
                | Self::UINTEGER
                | Self::UBIGINT
                | Self::FLOAT
                | Self::DOUBLE
        )
    }
}

impl fmt::Debug for TypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypeId({})", self.0)
    }
}

/// An owned logical type handle.
pub struct LogicalType(sys::duckdb_v2_logical_type_handle);

// SAFETY: a logical type is read-only once constructed (api_spec logical_type handle docs), so a
// handle can be shared and dropped from any thread.
unsafe impl Send for LogicalType {}
unsafe impl Sync for LogicalType {}

impl LogicalType {
    /// # Safety
    ///
    /// `handle` must be an owned logical type handle; it is destroyed on drop.
    pub(crate) unsafe fn from_raw(handle: sys::duckdb_v2_logical_type_handle) -> Self {
        Self(handle)
    }

    pub fn raw(&self) -> sys::duckdb_v2_logical_type_handle {
        self.0
    }

    pub fn id(&self) -> TypeId {
        let mut id = sys::DUCKDB_V2_LOGICAL_TYPE_ID_INVALID;
        // A valid handle always has an id; on failure INVALID is the answer.
        let _ = check(|err| unsafe { ffi!(duckdb_v2_logical_type_get_id(self.0, &mut id, err)) });
        TypeId(id)
    }

    /// The type as SQL text, e.g. `BIGINT`, `DECIMAL(2,1)` or `CHART`.
    pub fn to_text(&self) -> String {
        let mut len: sys::idx_t = 0;
        if check(|err| unsafe {
            ffi!(duckdb_v2_logical_type_to_text(
                self.0,
                ptr::null_mut(),
                0,
                &mut len,
                err
            ))
        })
        .is_err()
        {
            return "?".to_string();
        }
        let mut buf = vec![0u8; len as usize + 1];
        if check(|err| unsafe {
            ffi!(duckdb_v2_logical_type_to_text(
                self.0,
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len() as sys::idx_t,
                &mut len,
                err
            ))
        })
        .is_err()
        {
            return "?".to_string();
        }
        buf.truncate(len as usize);
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Value parameter `index` of the type: `DECIMAL`'s width and scale, a `LIST`'s element
    /// type, ... (see `logical_type_get_param`).
    pub fn param(&self, index: usize) -> Result<Value> {
        let mut name = sys::duckdb_v2_identifier_t::default();
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_logical_type_get_param(
                self.0,
                index as sys::idx_t,
                &mut name,
                &mut out,
                err
            ))
        })?;
        Ok(Value(out))
    }

    /// The width and scale of a `DECIMAL` type.
    pub fn decimal_width_scale(&self) -> Result<(u8, u8)> {
        let width = self.param(0)?.as_i64()?;
        let scale = self.param(1)?.as_i64()?;
        Ok((width as u8, scale as u8))
    }

    /// Deep equality, which tells a custom type from its base type.
    pub fn same_as(&self, other: &LogicalType) -> bool {
        let mut eq = false;
        check(|err| unsafe {
            ffi!(duckdb_v2_logical_type_is_equal(
                self.0, other.0, &mut eq, err
            ))
        })
        .is_ok()
            && eq
    }
}

impl Clone for LogicalType {
    fn clone(&self) -> Self {
        let mut out = ptr::null_mut();
        check(|err| unsafe { ffi!(duckdb_v2_logical_type_copy(self.0, &mut out, err)) })
            .expect("copying a logical type");
        Self(out)
    }
}

impl Drop for LogicalType {
    fn drop(&mut self) {
        // SAFETY: owned handle, destroyed once.
        unsafe {
            ffi!(duckdb_v2_logical_type_destroy(&mut self.0));
        }
    }
}

impl fmt::Debug for LogicalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

/// An owned SQL value handle.
pub struct Value(sys::duckdb_v2_value_handle);

// SAFETY: values are self-contained owned objects with no thread affinity.
unsafe impl Send for Value {}
unsafe impl Sync for Value {}

impl Value {
    /// # Safety
    ///
    /// `handle` must be an owned value handle; it is destroyed on drop.
    pub(crate) unsafe fn from_raw(handle: sys::duckdb_v2_value_handle) -> Self {
        Self(handle)
    }

    pub fn raw(&self) -> sys::duckdb_v2_value_handle {
        self.0
    }

    pub fn is_null(&self) -> bool {
        let mut null = false;
        let _ = check(|err| unsafe { ffi!(duckdb_v2_value_is_null(self.0, &mut null, err)) });
        null
    }

    pub fn logical_type(&self) -> Result<LogicalType> {
        let mut out = ptr::null_mut();
        check(|err| unsafe { ffi!(duckdb_v2_value_get_logical_type(self.0, &mut out, err)) })?;
        Ok(LogicalType(out))
    }

    /// The value as text, for `VARCHAR` values.
    pub fn as_str(&self) -> Result<String> {
        let mut out = sys::duckdb_v2_str::default();
        check(|err| unsafe { ffi!(duckdb_v2_value_get_varchar(self.0, &mut out, err)) })?;
        // SAFETY: borrowed from the value, which outlives this copy.
        Ok(String::from_utf8_lossy(unsafe { sys::str_bytes(out) }).into_owned())
    }

    pub fn as_i64(&self) -> Result<i64> {
        let mut out = 0i64;
        check(|err| unsafe { ffi!(duckdb_v2_value_get_bigint(self.0, &mut out, err)) })?;
        Ok(out)
    }

    pub fn as_f64(&self) -> Result<f64> {
        let mut out = 0f64;
        check(|err| unsafe { ffi!(duckdb_v2_value_get_double(self.0, &mut out, err)) })?;
        Ok(out)
    }

    pub fn as_bool(&self) -> Result<bool> {
        let mut out = false;
        check(|err| unsafe { ffi!(duckdb_v2_value_get_bool(self.0, &mut out, err)) })?;
        Ok(out)
    }

    /// The value rendered as SQL text, as `value::VARCHAR` would.
    pub fn to_display(&self) -> String {
        let mut len: sys::idx_t = 0;
        if check(|err| unsafe {
            ffi!(duckdb_v2_value_to_string(
                self.0,
                ptr::null_mut(),
                0,
                &mut len,
                err
            ))
        })
        .is_err()
        {
            return "?".to_string();
        }
        let mut buf = vec![0u8; len as usize + 1];
        if check(|err| unsafe {
            ffi!(duckdb_v2_value_to_string(
                self.0,
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len() as sys::idx_t,
                &mut len,
                err
            ))
        })
        .is_err()
        {
            return "?".to_string();
        }
        buf.truncate(len as usize);
        String::from_utf8_lossy(&buf).into_owned()
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        // SAFETY: owned handle, destroyed once.
        unsafe {
            ffi!(duckdb_v2_value_destroy(&mut self.0));
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display())
    }
}
