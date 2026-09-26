//! Thin wrappers over the parts of DuckDB's C API that duckdb-rs doesn't cover: reading
//! input vectors, custom logical types and cast functions.

use duckdb::{
    core::{LogicalTypeHandle, LogicalTypeId},
    ffi,
};
use std::{
    error::Error,
    ffi::{CString, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

pub type BoxError = Box<dyn Error>;

/// Name of the logical type alias for chart values.
pub const CHART_TYPE: &str = "CHART";

/// An owned raw logical type, destroyed on drop.
pub struct RawType(ffi::duckdb_logical_type);

impl RawType {
    pub fn new(ty: ffi::DUCKDB_TYPE) -> Self {
        Self(unsafe { ffi::duckdb_create_logical_type(ty) })
    }

    pub fn varchar() -> Self {
        Self::new(ffi::DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR)
    }

    /// `CHART`: a BLOB with the [`CHART_TYPE`] alias.
    pub fn chart() -> Self {
        let ty = Self::new(ffi::DUCKDB_TYPE_DUCKDB_TYPE_BLOB);
        let alias = CString::new(CHART_TYPE).unwrap();
        unsafe { ffi::duckdb_logical_type_set_alias(ty.0, alias.as_ptr()) };
        ty
    }

    pub fn as_ptr(&self) -> ffi::duckdb_logical_type {
        self.0
    }
}

impl Drop for RawType {
    fn drop(&mut self) {
        unsafe { ffi::duckdb_destroy_logical_type(&mut self.0) };
    }
}

/// `CHART` as a duckdb-rs type handle, for scalar function signatures.
pub fn chart_type_handle() -> LogicalTypeHandle {
    let mut ty: LogicalTypeHandle = LogicalTypeId::Blob.into();
    ty.set_alias(CHART_TYPE);
    ty
}

/// A read-only view of one column of a flat data chunk.
///
/// DuckDB flattens the inputs of C API scalar, aggregate and cast functions, so each column
/// is a plain array plus an optional validity mask.
pub struct Column {
    data: *mut c_void,
    validity: *mut u64,
}

impl Column {
    /// # Safety
    /// `vector` must be a valid flat vector that outlives the returned view.
    pub unsafe fn from_vector(vector: ffi::duckdb_vector) -> Self {
        unsafe {
            Self {
                data: ffi::duckdb_vector_get_data(vector),
                validity: ffi::duckdb_vector_get_validity(vector),
            }
        }
    }

    /// All columns of a chunk.
    ///
    /// # Safety
    /// `chunk` must be a valid, flat data chunk that outlives the returned views.
    pub unsafe fn from_chunk(chunk: ffi::duckdb_data_chunk) -> Vec<Self> {
        unsafe {
            (0..ffi::duckdb_data_chunk_get_column_count(chunk))
                .map(|i| Self::from_vector(ffi::duckdb_data_chunk_get_vector(chunk, i)))
                .collect()
        }
    }

    pub fn is_valid(&self, row: usize) -> bool {
        self.validity.is_null()
            || unsafe { ffi::duckdb_validity_row_is_valid(self.validity, row as u64) }
    }

    /// # Safety
    /// `T` must match the column's physical type and `row` must be in range.
    pub unsafe fn get<T: Copy>(&self, row: usize) -> T {
        unsafe { *self.data.cast::<T>().add(row) }
    }

    /// The bytes of a VARCHAR or BLOB value.
    ///
    /// # Safety
    /// The column must be VARCHAR or BLOB and `row` must be in range.
    pub unsafe fn bytes(&self, row: usize) -> &[u8] {
        unsafe {
            let s = self.data.cast::<ffi::duckdb_string_t>().add(row);
            let len = ffi::duckdb_string_t_length(*s) as usize;
            let data = ffi::duckdb_string_t_data(s);
            std::slice::from_raw_parts(data.cast(), len)
        }
    }

    /// # Safety
    /// Same as [`Column::bytes`].
    pub unsafe fn string(&self, row: usize) -> String {
        String::from_utf8_lossy(unsafe { self.bytes(row) }).into_owned()
    }
}

/// A LIST column: per-row `(offset, length)` entries into a flat child column.
pub struct ListColumn {
    entries: *const ffi::duckdb_list_entry,
    pub child: Column,
}

impl ListColumn {
    /// Column `index` of `chunk`, which must be a flat LIST vector.
    ///
    /// # Safety
    /// `chunk` must be a valid, flat data chunk that outlives the returned view.
    pub unsafe fn from_chunk(chunk: ffi::duckdb_data_chunk, index: usize) -> Self {
        unsafe {
            let vector = ffi::duckdb_data_chunk_get_vector(chunk, index as u64);
            Self {
                entries: ffi::duckdb_vector_get_data(vector).cast(),
                child: Column::from_vector(ffi::duckdb_list_vector_get_child(vector)),
            }
        }
    }

    /// The child indices of row `row`'s list.
    ///
    /// # Safety
    /// `row` must be in range and valid.
    pub unsafe fn range(&self, row: usize) -> std::ops::Range<usize> {
        let entry = unsafe { *self.entries.add(row) };
        entry.offset as usize..(entry.offset + entry.length) as usize
    }
}

fn check(state: ffi::duckdb_state, what: &str) -> Result<(), BoxError> {
    if state == ffi::duckdb_state_DuckDBSuccess {
        Ok(())
    } else {
        Err(format!("failed to register {what}").into())
    }
}

/// Runs `f`, turning errors and panics into a message for DuckDB.
fn guarded(f: impl FnOnce() -> Result<(), BoxError>) -> Result<(), CString> {
    let message = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(e)) => e.to_string(),
        Err(payload) => match payload.downcast_ref::<&str>() {
            Some(s) => format!("duckers panicked: {s}"),
            None => match payload.downcast_ref::<String>() {
                Some(s) => format!("duckers panicked: {s}"),
                None => "duckers panicked".to_string(),
            },
        },
    };
    Err(CString::new(message.replace('\0', " ")).unwrap())
}

/// Registers the `CHART` type alias on `con`.
///
/// # Safety
/// `con` must be a valid connection.
pub unsafe fn register_chart_type(con: ffi::duckdb_connection) -> Result<(), BoxError> {
    let ty = RawType::chart();
    check(
        unsafe { ffi::duckdb_register_logical_type(con, ty.as_ptr(), ptr::null_mut()) },
        "type CHART",
    )
}

/// Registers an explicit cast from `source` to `target`, implemented by `F`.
///
/// # Safety
/// `con` must be a valid connection.
pub unsafe fn register_cast<F: CastImpl>(
    con: ffi::duckdb_connection,
    source: RawType,
    target: RawType,
) -> Result<(), BoxError> {
    unsafe {
        let mut cast = ffi::duckdb_create_cast_function();
        ffi::duckdb_cast_function_set_source_type(cast, source.as_ptr());
        ffi::duckdb_cast_function_set_target_type(cast, target.as_ptr());
        ffi::duckdb_cast_function_set_function(cast, Some(cast_callback::<F>));
        let state = ffi::duckdb_register_cast_function(con, cast);
        ffi::duckdb_destroy_cast_function(&mut cast);
        check(state, "cast")
    }
}

/// A cast callback body: converts `count` input rows into `output`.
pub trait CastImpl {
    fn cast(count: usize, input: &Column, output: &mut ffi::duckdb_vector) -> Result<(), BoxError>;
}

unsafe extern "C" fn cast_callback<F: CastImpl>(
    info: ffi::duckdb_function_info,
    count: ffi::idx_t,
    input: ffi::duckdb_vector,
    mut output: ffi::duckdb_vector,
) -> bool {
    let input = unsafe { Column::from_vector(input) };
    match guarded(|| F::cast(count as usize, &input, &mut output)) {
        Ok(()) => true,
        Err(message) => {
            unsafe { ffi::duckdb_cast_function_set_error(info, message.as_ptr()) };
            false
        }
    }
}
