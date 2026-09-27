//! Reading argument vectors and writing result vectors.

use std::marker::PhantomData;
use std::ptr;

use duckers_sys::{self as sys, ffi};

use super::error::{Error, Result, check};
use super::types::{LogicalType, TypeId, Value};

/// A read-only view of an input vector in DuckDB's unified format: data, optional validity mask
/// and optional selection vector.
///
/// DuckDB v2 hands callbacks (scalar exec, aggregate update, casts) vectors in whatever
/// representation they have. Flat, constant and dictionary vectors are read in place through the
/// selection vector; compressed and sequence vectors (`range()` produces the latter) are flattened
/// first, since the view rejects them.
pub struct InputVector<'a> {
    handle: sys::duckdb_v2_vector_handle,
    view: sys::duckdb_v2_vector_view,
    rows: usize,
    ty: LogicalType,
    type_id: TypeId,
    /// `(width, scale)` of a `DECIMAL` vector.
    decimal: Option<(u8, u8)>,
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
        let ty = unsafe { LogicalType::from_raw(ty) };
        let type_id = ty.id();
        let decimal = if type_id == TypeId::DECIMAL {
            Some(ty.decimal_width_scale()?)
        } else {
            None
        };
        Ok(Self {
            handle,
            view,
            rows,
            ty,
            type_id,
            decimal,
            _chunk: PhantomData,
        })
    }

    /// The vector's logical type.
    pub fn logical_type(&self) -> &LogicalType {
        &self.ty
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

    /// The value at `row` of a numeric vector (integers of any width, `FLOAT`, `DOUBLE`,
    /// `DECIMAL`), converted to `f64`. Other types are an error.
    pub fn f64(&self, row: usize) -> Result<f64> {
        if let Some((_, scale)) = self.decimal {
            return Ok(self.decimal_unscaled(row)? as f64 / 10f64.powi(i32::from(scale)));
        }
        // SAFETY: each arm reads the physical type the type id implies.
        unsafe {
            Ok(match self.type_id {
                TypeId::HUGEINT => self.hugeint(row) as f64,
                TypeId::UHUGEINT => self.uhugeint(row) as f64,
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

    /// The unscaled integer of a `DECIMAL` vector at `row`: the stored value, whose physical type
    /// follows from the width (`i16` up to 4 digits, `i32` up to 9, `i64` up to 18, `i128` up
    /// to 38).
    pub fn decimal_unscaled(&self, row: usize) -> Result<i128> {
        let Some((width, _)) = self.decimal else {
            return Err(Error::internal(format!(
                "decimal_unscaled() on a vector of type id {:?}",
                self.type_id
            )));
        };
        // SAFETY: the arm matches the storage tier the C API documents for the width.
        unsafe {
            Ok(match width {
                0..=4 => i128::from(self.get::<i16>(row)),
                5..=9 => i128::from(self.get::<i32>(row)),
                10..=18 => i128::from(self.get::<i64>(row)),
                _ => self.hugeint(row),
            })
        }
    }

    /// The scale of a `DECIMAL` vector.
    pub fn decimal_scale(&self) -> Option<u8> {
        self.decimal.map(|(_, scale)| scale)
    }

    /// # Safety
    ///
    /// The vector's physical type must be `hugeint_t`.
    unsafe fn hugeint(&self, row: usize) -> i128 {
        // SAFETY: guaranteed by the caller.
        let h = unsafe { self.get::<sys::duckdb_v2_hugeint_t>(row) };
        (i128::from(h.upper) << 64) | i128::from(h.lower)
    }

    /// # Safety
    ///
    /// The vector's physical type must be `uhugeint_t`.
    unsafe fn uhugeint(&self, row: usize) -> u128 {
        // SAFETY: guaranteed by the caller.
        let h = unsafe { self.get::<sys::duckdb_v2_uhugeint_t>(row) };
        (u128::from(h.upper) << 64) | u128::from(h.lower)
    }

    /// The value at `row` of an integer vector of any width as `i128`. `UHUGEINT` values above
    /// `i128::MAX` are an error.
    pub fn i128(&self, row: usize) -> Result<i128> {
        // SAFETY: each arm reads the physical type the type id implies.
        unsafe {
            Ok(match self.type_id {
                TypeId::HUGEINT => self.hugeint(row),
                TypeId::UHUGEINT => i128::try_from(self.uhugeint(row))
                    .map_err(|_| Error::invalid_input("UHUGEINT value out of HUGEINT range"))?,
                TypeId::UBIGINT => i128::from(self.get::<u64>(row)),
                _ => i128::from(self.i64(row)?),
            })
        }
    }

    /// The cell at `row` as an owned value, for any type and representation. Slow: one
    /// allocation per call.
    pub fn value(&self, row: usize) -> Result<Value> {
        let mut out = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_vector_get_value(
                self.handle,
                row as sys::idx_t,
                &mut out,
                err
            ))
        })?;
        // SAFETY: vector_get_value returns an owned value.
        Ok(unsafe { Value::from_raw(out) })
    }

    /// The element range of a `LIST` vector at `row`, as offsets into [`list_child`].
    ///
    /// [`list_child`]: Self::list_child
    pub fn list_entry(&self, row: usize) -> Result<std::ops::Range<usize>> {
        if self.type_id != TypeId::LIST {
            return Err(Error::internal(format!(
                "list_entry() on a vector of type id {:?}",
                self.type_id
            )));
        }
        // SAFETY: LIST vectors store `duckdb_v2_list_entry` elements.
        let entry = unsafe { self.get::<sys::duckdb_v2_list_entry>(row) };
        let start = entry.offset as usize;
        Ok(start..start + entry.length as usize)
    }

    /// The element vector of a `LIST` vector, holding the elements of every row.
    pub fn list_child(&self) -> Result<InputVector<'_>> {
        if self.type_id != TypeId::LIST {
            return Err(Error::internal(format!(
                "list_child() on a vector of type id {:?}",
                self.type_id
            )));
        }
        let mut child = ptr::null_mut();
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_child(self.handle, 0, &mut child, err)) })?;
        let mut size: sys::idx_t = 0;
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_size(child, &mut size, err)) })?;
        // SAFETY: the child lives as long as the parent's chunk, and holds `size` elements.
        unsafe { InputVector::from_raw(child, size as usize) }
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

    /// Writes a list of byte strings into a `LIST` result vector whose elements are `VARCHAR`,
    /// `BLOB` or a custom type over `BLOB`: the items are appended to the element vector, and
    /// `row` points at them.
    pub fn set_list_bytes<B: AsRef<[u8]>>(&mut self, row: usize, items: &[B]) -> Result<()> {
        let mut child = ptr::null_mut();
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_child(self.handle, 0, &mut child, err)) })?;
        let mut offset: sys::idx_t = 0;
        check(|err| unsafe { ffi!(duckdb_v2_vector_get_size(child, &mut offset, err)) })?;
        let length = items.len() as sys::idx_t;
        check(|err| unsafe { ffi!(duckdb_v2_vector_set_size(child, offset + length, err)) })?;
        {
            // SAFETY: the element vector belongs to this result vector, and the reservation above
            // covers rows `offset..offset + length`.
            let mut elements = unsafe { OutputVector::from_raw(child)? };
            for (i, item) in items.iter().enumerate() {
                elements.set_bytes(offset as usize + i, item.as_ref())?;
            }
        }
        let entry = sys::duckdb_v2_list_entry { offset, length };
        // SAFETY: LIST vectors store `duckdb_v2_list_entry` elements.
        unsafe { self.set(row, entry) };
        Ok(())
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
