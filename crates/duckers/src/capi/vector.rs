//! Reading argument vectors and writing result vectors.

use std::marker::PhantomData;
use std::ptr;

use duckers_sys::{self as sys, ffi};

use super::error::{Error, Result, check};
use super::types::{LogicalType, TypeId};

/// A read-only view of an input vector in DuckDB's unified format: data, optional validity mask
/// and optional selection vector.
///
/// DuckDB v2 hands callbacks (scalar exec, aggregate update, casts) vectors in whatever
/// representation they have. Flat, constant and dictionary vectors are read in place through the
/// selection vector; compressed and sequence vectors (`range()` produces the latter) are flattened
/// first, since the view rejects them.
pub struct InputVector<'a> {
    view: sys::duckdb_v2_vector_view,
    rows: usize,
    type_id: TypeId,
    _chunk: PhantomData<&'a ()>,
}

impl InputVector<'_> {
    /// # Safety
    ///
    /// `handle` must be a vector DuckDB passed to the running callback, holding at least `rows`
    /// logical rows, and it must stay alive and unmodified while the view is used.
    pub(crate) unsafe fn from_raw(
        handle: sys::duckdb_v2_vector_handle,
        rows: usize,
    ) -> Result<Self> {
        let mut kind = sys::DUCKDB_V2_VECTOR_TYPE_OTHER;
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_vector_type(handle, &mut kind, err)) })?;
        if kind == sys::DUCKDB_V2_VECTOR_TYPE_OTHER {
            check(|err| unsafe { ffi!(duckdb_v2_vector_flatten(handle, err)) })?;
        }
        let mut view = sys::duckdb_v2_vector_view::default();
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_view(handle, &mut view, err)) })?;
        let mut ty = ptr::null_mut();
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_logical_type(handle, &mut ty, err)) })?;
        // SAFETY: get_logical_type returns an owned handle.
        let type_id = unsafe { LogicalType::from_raw(ty) }.id();
        Ok(Self {
            view,
            rows,
            type_id,
            _chunk: PhantomData,
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The type id of the vector's logical type. Custom types report their base id (`BLOB`).
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    #[inline]
    fn physical(&self, row: usize) -> usize {
        debug_assert!(row < self.rows);
        if self.view.sel.is_null() {
            row
        } else {
            // SAFETY: the selection vector covers every logical row of the view.
            unsafe { *self.view.sel.add(row) as usize }
        }
    }

    #[inline]
    pub fn is_valid(&self, row: usize) -> bool {
        if self.view.validity.is_null() {
            return true;
        }
        let i = self.physical(row);
        // SAFETY: the mask has one bit per physical row.
        let word = unsafe { *self.view.validity.add(i / 64) };
        word & (1u64 << (i % 64)) != 0
    }

    /// The fixed-width value at `row`. `T` must match the vector's physical layout: `i64` for
    /// `BIGINT`, `f64` for `DOUBLE`, `i32` for `DATE`, and so on.
    ///
    /// # Safety
    ///
    /// The vector's element type must be `T`.
    #[inline]
    pub unsafe fn get<T: Copy>(&self, row: usize) -> T {
        let i = self.physical(row);
        // SAFETY: guaranteed by the caller; the data array covers every physical row.
        unsafe { *self.view.data.cast::<T>().add(i) }
    }

    /// The bytes at `row` of a `VARCHAR`, `BLOB` or custom-over-`BLOB` vector.
    pub fn bytes(&self, row: usize) -> Result<&[u8]> {
        if !matches!(self.type_id, TypeId::VARCHAR | TypeId::BLOB) {
            return Err(Error::internal(format!(
                "bytes() on a vector of type id {:?}",
                self.type_id
            )));
        }
        let i = self.physical(row);
        // SAFETY: VARCHAR and BLOB vectors store `duckdb_v2_bytes` elements; the out-of-line
        // bytes live as long as the vector.
        unsafe {
            let b = &*self.view.data.cast::<sys::duckdb_v2_bytes>().add(i);
            let len = b.value.inlined.length as usize;
            if len <= sys::DUCKDB_V2_BYTES_INLINE_LENGTH as usize {
                Ok(std::slice::from_raw_parts(
                    b.value.inlined.inlined.as_ptr().cast::<u8>(),
                    len,
                ))
            } else {
                Ok(std::slice::from_raw_parts(
                    b.value.pointer.ptr.cast::<u8>(),
                    len,
                ))
            }
        }
    }

    /// The text at `row` of a `VARCHAR` vector.
    pub fn str(&self, row: usize) -> Result<&str> {
        std::str::from_utf8(self.bytes(row)?)
            .map_err(|e| Error::invalid_input(format!("invalid UTF-8 in VARCHAR: {e}")))
    }

    /// The value at `row` of a plain numeric vector (integers of any width, `FLOAT`, `DOUBLE`),
    /// converted to `f64`. Other types (`DECIMAL`, `HUGEINT`, ...) are an error.
    pub fn f64(&self, row: usize) -> Result<f64> {
        // SAFETY: each arm reads the physical type the type id implies.
        unsafe {
            Ok(match self.type_id {
                TypeId::TINYINT => self.get::<i8>(row) as f64,
                TypeId::SMALLINT => self.get::<i16>(row) as f64,
                TypeId::INTEGER => self.get::<i32>(row) as f64,
                TypeId::BIGINT => self.get::<i64>(row) as f64,
                TypeId::UTINYINT => self.get::<u8>(row) as f64,
                TypeId::USMALLINT => self.get::<u16>(row) as f64,
                TypeId::UINTEGER => self.get::<u32>(row) as f64,
                TypeId::UBIGINT => self.get::<u64>(row) as f64,
                TypeId::FLOAT => self.get::<f32>(row) as f64,
                TypeId::DOUBLE => self.get::<f64>(row),
                other => {
                    return Err(Error::internal(format!(
                        "f64() on a vector of type id {other:?}"
                    )));
                }
            })
        }
    }

    /// The value at `row` of an integer vector as `i64`.
    pub fn i64(&self, row: usize) -> Result<i64> {
        // SAFETY: each arm reads the physical type the type id implies.
        unsafe {
            Ok(match self.type_id {
                TypeId::TINYINT => self.get::<i8>(row) as i64,
                TypeId::SMALLINT => self.get::<i16>(row) as i64,
                TypeId::INTEGER => self.get::<i32>(row) as i64,
                TypeId::BIGINT => self.get::<i64>(row),
                TypeId::UTINYINT => self.get::<u8>(row) as i64,
                TypeId::USMALLINT => self.get::<u16>(row) as i64,
                TypeId::UINTEGER => self.get::<u32>(row) as i64,
                TypeId::UBIGINT => i64::try_from(self.get::<u64>(row))
                    .map_err(|_| Error::invalid_input("UBIGINT value out of BIGINT range"))?,
                other => {
                    return Err(Error::internal(format!(
                        "i64() on a vector of type id {other:?}"
                    )));
                }
            })
        }
    }
}

/// A result vector being written, one row at a time.
///
/// Writes go to flat storage. A constant result vector (DuckDB uses one when every argument is
/// constant) holds a single row, and writing row 0 fills it.
pub struct OutputVector<'a> {
    handle: sys::duckdb_v2_vector_handle,
    data: *mut std::ffi::c_void,
    constant: bool,
    validity: *mut u64,
    arena: sys::duckdb_v2_arena_handle,
    _chunk: PhantomData<&'a mut ()>,
}

impl OutputVector<'_> {
    /// # Safety
    ///
    /// `handle` must be the result vector DuckDB passed to the running callback.
    pub(crate) unsafe fn from_raw(handle: sys::duckdb_v2_vector_handle) -> Result<Self> {
        let mut kind = sys::DUCKDB_V2_VECTOR_TYPE_OTHER;
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_vector_type(handle, &mut kind, err)) })?;
        let constant = kind == sys::DUCKDB_V2_VECTOR_TYPE_CONSTANT;
        if !constant && kind != sys::DUCKDB_V2_VECTOR_TYPE_FLAT {
            check(|err| unsafe { ffi!(duckdb_v2_vector_flatten(handle, err)) })?;
        }
        let mut data = ptr::null_mut();
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_data_mutable(handle, &mut data, err)) })?;
        Ok(Self {
            handle,
            data,
            constant,
            validity: ptr::null_mut(),
            arena: ptr::null_mut(),
            _chunk: PhantomData,
        })
    }

    /// Marks `row` NULL. The result types duckers writes are not STRUCT or ARRAY, so clearing the
    /// row's validity bit is enough (`vector_set_null` would also clear nested children, and it
    /// bounds-checks against a vector size that result vectors do not carry).
    pub fn set_null(&mut self, row: usize) -> Result<()> {
        if self.constant {
            return check(|err| unsafe {
                ffi!(duckdb_v2_vector_constant_set_valid(self.handle, false, err))
            });
        }
        if self.validity.is_null() {
            let mut v = ptr::null_mut();
            check(|err| unsafe {
                ffi!(duckdb_v2_vector_flat_get_validity_mutable(
                    self.handle,
                    &mut v,
                    err
                ))
            })?;
            self.validity = v;
        }
        // SAFETY: the mask has one bit per row of the vector's capacity.
        unsafe { *self.validity.add(row / 64) &= !(1u64 << (row % 64)) };
        Ok(())
    }

    /// Writes a fixed-width value. `T` must match the vector's physical layout.
    ///
    /// # Safety
    ///
    /// The vector's element type must be `T`, and `row` must be within the vector.
    pub unsafe fn set<T: Copy>(&mut self, row: usize, value: T) {
        // SAFETY: guaranteed by the caller.
        unsafe { *self.data.cast::<T>().add(row) = value };
    }

    /// Writes bytes into a `VARCHAR`, `BLOB` or custom-over-`BLOB` result vector. Strings must be
    /// valid UTF-8 for `VARCHAR`, which [`set_str`](Self::set_str) guarantees.
    pub fn set_bytes(&mut self, row: usize, bytes: &[u8]) -> Result<()> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| Error::invalid_input("value larger than 4 GiB"))?;
        let mut value = sys::duckdb_v2_bytes::default_zeroed();
        if bytes.len() <= sys::DUCKDB_V2_BYTES_INLINE_LENGTH as usize {
            // SAFETY: writing the inlined arm of a zeroed union.
            unsafe {
                value.value.inlined.length = len;
                ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    value.value.inlined.inlined.as_mut_ptr().cast::<u8>(),
                    bytes.len(),
                );
            }
        } else {
            if self.arena.is_null() {
                let mut arena = ptr::null_mut();
                check(|err| unsafe {
                    ffi!(duckdb_v2_vector_get_arena(self.handle, &mut arena, err))
                })?;
                self.arena = arena;
            }
            let mut dst: *mut u8 = ptr::null_mut();
            check(|err| unsafe {
                ffi!(duckdb_v2_arena_allocate(
                    self.arena,
                    bytes.len() as sys::idx_t,
                    &mut dst,
                    err
                ))
            })?;
            // SAFETY: the arena handed out `len` writable bytes that live as long as the vector.
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
                value.value.pointer.length = len;
                ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    value.value.pointer.prefix.as_mut_ptr().cast::<u8>(),
                    4,
                );
                value.value.pointer.ptr = dst.cast();
            }
        }
        // SAFETY: string vectors store `duckdb_v2_bytes` elements.
        unsafe { self.set(row, value) };
        Ok(())
    }

    pub fn set_str(&mut self, row: usize, s: &str) -> Result<()> {
        self.set_bytes(row, s.as_bytes())
    }
}

trait ZeroedBytes {
    fn default_zeroed() -> Self;
}

impl ZeroedBytes for sys::duckdb_v2_bytes {
    fn default_zeroed() -> Self {
        // SAFETY: all-zero is a valid (empty, inlined) bytes value.
        unsafe { std::mem::zeroed() }
    }
}
