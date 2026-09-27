//! Cast functions.

use std::ffi::c_void;
use std::ptr;

use duckers_sys::{self as sys, ffi};

use super::error::{Result, check, guard};
use super::function::opaque;
use super::scalar::WriteCell;
use super::types::LogicalType;
use super::vector::{InputVector, OutputVector};

/// Converts one input row. `Ok(None)` writes NULL.
pub type CastRow = dyn Fn(&InputVector<'_>, usize) -> Result<Option<Vec<u8>>> + Send + Sync;

struct CastImpl {
    f: Box<CastRow>,
}

/// Registers a cast from `source` to `target` computed row by row.
///
/// NULL rows stay NULL. A row that fails raises the error under `CAST`, and becomes NULL under
/// `TRY_CAST`. The target must be a string-backed type (`VARCHAR`, `BLOB` or a custom type over
/// `BLOB`), since every row is written as bytes.
pub(crate) fn register_bytes_cast(
    extension: sys::duckdb_v2_extension_handle,
    source: &LogicalType,
    target: &LogicalType,
    f: Box<CastRow>,
) -> Result<()> {
    let mut handle = ptr::null_mut();
    check(|err| unsafe {
        ffi!(duckdb_v2_cast_function_create_with_extension(
            extension,
            &mut handle,
            err
        ))
    })?;
    let result = (|| {
        check(|err| unsafe {
            ffi!(duckdb_v2_cast_function_set_source_type(
                handle,
                source.raw(),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_cast_function_set_target_type(
                handle,
                target.raw(),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_cast_function_set_exec_callback(
                handle,
                Some(cast_exec),
                err
            ))
        })?;
        let mut data = opaque(CastImpl { f });
        check(|err| unsafe {
            ffi!(duckdb_v2_cast_function_set_user_data(
                handle, &mut data, err
            ))
        })?;
        check(|err| unsafe { ffi!(duckdb_v2_cast_function_register(handle, err)) })
    })();
    // SAFETY: the handle is ours; destroying it leaves the registered cast intact.
    unsafe { ffi!(duckdb_v2_cast_function_destroy(&mut handle)) };
    result
}

unsafe extern "C" fn cast_exec(
    info: sys::duckdb_v2_cast_function_exec_info_handle,
    _context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live exec info and error slot; the vectors stay valid for the call.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_cast_function_exec_get_user_data(
                    info, &mut user, e
                ))
            })?;
            let imp = &*user.cast::<CastImpl>();
            let mut rows: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_cast_function_exec_get_row_count(
                    info, &mut rows, e
                ))
            })?;
            let mut mode = sys::DUCKDB_V2_CAST_MODE_NORMAL;
            check(|e| ffi!(duckdb_v2_cast_function_exec_get_mode(info, &mut mode, e)))?;
            let try_cast = mode == sys::DUCKDB_V2_CAST_MODE_TRY;
            let mut input = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_cast_function_exec_get_input(info, &mut input, e)))?;
            let mut output = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_cast_function_exec_get_output(
                    info,
                    &mut output,
                    e
                ))
            })?;
            let input = InputVector::from_raw(input, rows as usize)?;
            let mut out = OutputVector::from_raw(output)?;
            for row in 0..rows as usize {
                if !input.is_valid(row) {
                    out.set_null(row)?;
                    continue;
                }
                match (imp.f)(&input, row) {
                    Ok(Some(bytes)) => bytes.write(&mut out, row)?,
                    Ok(None) => out.set_null(row)?,
                    Err(_) if try_cast => out.set_null(row)?,
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })
    }
}
