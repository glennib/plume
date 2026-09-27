//! duckers: plotters charts for DuckDB v2, as a loadable extension on the stable v2 C API.

pub mod capi;
mod spike;

/// The extension entrypoint. DuckDB derives the symbol name from the file name (`duckers`) and
/// the footer's ABI (`C_STRUCT` with a `v2.x.y` C API version selects `<name>_init_c_api_v2`).
///
/// # Safety
///
/// Called by DuckDB's loader with its extension input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn duckers_init_c_api_v2(input: *mut duckers_sys::duckdb_v2_extension_input) {
    // SAFETY: forwarded from the loader.
    unsafe { capi::entrypoint(input, spike::load) }
}
