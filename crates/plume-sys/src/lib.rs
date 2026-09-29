//! Raw bindings to the DuckDB v2 C extension API.
//!
//! [`bindings`] is bindgen output over the vendored `duckdb_extension_v2.h` (C API `v2.0.0`, from
//! the DuckDB build named in `vendor/VERSION`). It holds the types, the constants and the
//! function-pointer table [`duckdb_ext_api_v2`], but none of the free `duckdb_v2_*` functions:
//! a loadable extension cannot link against them, since the host process exports no DuckDB
//! symbols. The C header reaches the table through indirection macros, which bindgen cannot
//! translate, so this crate stores the table that `get_api` returns ([`init_api`]) and the
//! [`ffi!`] macro calls through it.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::all)]

mod bindings;

use std::ffi::CStr;
use std::sync::OnceLock;

pub use bindings::*;

/// The C API version this crate's table layout corresponds to, as passed to `get_api` and written
/// into the extension metadata footer.
pub const C_API_VERSION: &CStr = c"v2.0.0";

static API: OnceLock<duckdb_ext_api_v2> = OnceLock::new();

/// Fetches the function-pointer table for [`C_API_VERSION`] from the loader and stores it.
///
/// Returns `false` when DuckDB cannot serve that version. The loader has then already recorded
/// why, and the entrypoint must return without touching the error slot.
///
/// Every load must call it: the loader checks that `get_api` ran during the entrypoint and fails
/// the load with a fatal error otherwise. A process that loads the extension into several
/// databases gets the same table each time, so the first stored copy stays.
///
/// # Safety
///
/// `input` must be the pointer DuckDB passed to the extension entrypoint.
pub unsafe fn init_api(input: *const duckdb_v2_extension_input) -> bool {
    // SAFETY: the caller passes the loader's input struct, valid for the entrypoint call.
    let input = unsafe { &*input };
    let Some(get_api) = input.get_api else {
        return false;
    };
    // SAFETY: get_api is DuckDB's; it returns NULL or a pointer to a table of at least the
    // requested version's size, which is the size of `duckdb_ext_api_v2`.
    let table = unsafe { get_api(input.extension, C_API_VERSION.as_ptr()) };
    if table.is_null() {
        return false;
    }
    // SAFETY: non-null and laid out as `duckdb_ext_api_v2` (see above). Copy it, as the C
    // entrypoint macro does, so nothing depends on the lifetime of DuckDB's copy.
    let _ = API.set(unsafe { *table.cast::<duckdb_ext_api_v2>() });
    true
}

/// The stored function-pointer table.
///
/// # Panics
///
/// Panics when called before [`init_api`] succeeded.
#[inline]
pub fn api() -> &'static duckdb_ext_api_v2 {
    API.get()
        .expect("DuckDB C API table used before plume_sys::init_api")
}

/// Whether [`init_api`] has stored a table. False in plain `cargo test` runs, which have no host.
pub fn api_available() -> bool {
    API.get().is_some()
}

/// Calls a DuckDB C API function through the stored table: `ffi!(duckdb_v2_vector_flatten(v, err))`.
///
/// Expands to an unsafe call, so it must appear inside an `unsafe` block. Panics if the slot is
/// empty, which a table from a DuckDB that serves [`C_API_VERSION`] never has.
#[macro_export]
macro_rules! ffi {
    ($name:ident ( $($arg:expr),* $(,)? )) => {
        ($crate::api()
            .$name
            .expect(concat!(stringify!($name), " is missing from the DuckDB C API table")))($($arg),*)
    };
}

/// Builds a [`duckdb_v2_str`] view over a Rust string or byte slice. The view borrows `s`.
#[inline]
pub fn str_view(s: &[u8]) -> duckdb_v2_str {
    duckdb_v2_str {
        ptr: s.as_ptr().cast(),
        len: s.len() as idx_t,
    }
}

/// Reads a [`duckdb_v2_str`] view as bytes.
///
/// # Safety
///
/// `s.ptr` must point at `s.len` readable bytes for the lifetime `'a`, or `s.len` must be 0.
#[inline]
pub unsafe fn str_bytes<'a>(s: duckdb_v2_str) -> &'a [u8] {
    if s.len == 0 || s.ptr.is_null() {
        &[]
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts(s.ptr.cast(), s.len as usize) }
    }
}
