// src/wasm_lib.rs includes this file as `mod lib`, so module files are named explicitly and
// modules refer to each other with `super::` paths rather than `crate::`.
#[path = "capi.rs"]
mod capi;
#[path = "functions/mod.rs"]
mod functions;
#[path = "render.rs"]
mod render;
#[path = "spec.rs"]
mod spec;

use capi::{BoxError, RawType};
use duckdb::{
    Connection, Result,
    core::{DataChunkHandle, Inserter, LogicalTypeId},
    ffi,
    vscalar::{ScalarFunctionSignature, VScalar},
    vtab::arrow::WritableVector,
};
use std::{ffi::CString, ptr};

/// `duckers_version()`: the version of the loaded extension.
struct VersionScalar;

impl VScalar for VersionScalar {
    type State = ();

    fn invoke(
        _state: &Self::State,
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), BoxError> {
        let output = output.flat_vector();
        for i in 0..input.len() {
            output.insert(i, env!("CARGO_PKG_VERSION"));
        }
        Ok(())
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![ScalarFunctionSignature::exact(
            vec![],
            LogicalTypeId::Varchar.into(),
        )]
    }
}

/// Registers everything. The `CHART` type and its cast go through the raw C API, which
/// needs a `duckdb_connection`; functions use duckdb-rs.
fn register(con: &Connection, raw: ffi::duckdb_connection) -> Result<(), BoxError> {
    render::register_fonts()?;
    unsafe {
        capi::register_chart_type(raw)?;
        capi::register_cast::<functions::scalar::SummaryCast>(
            raw,
            RawType::chart(),
            RawType::varchar(),
        )?;
    }
    functions::series::register(con)?;
    functions::scalar::register(con)?;
    con.register_scalar_function::<VersionScalar>("duckers_version")?;
    Ok(())
}

/// The C API version this build requires. The Makefile sets it to `TARGET_DUCKDB_VERSION`.
const MIN_DUCKDB_VERSION: &str = match option_env!("DUCKDB_EXTENSION_MIN_DUCKDB_VERSION") {
    Some(v) => v,
    None => "v1.2.0",
};

/// # Safety
/// `info` and `access` must be the values DuckDB passed to the entrypoint.
unsafe fn init(
    info: ffi::duckdb_extension_info,
    access: *const ffi::duckdb_extension_access,
) -> Result<bool, BoxError> {
    unsafe {
        if !ffi::duckdb_rs_extension_api_init(info, access, MIN_DUCKDB_VERSION)? {
            // Most likely a C API version mismatch; DuckDB reports it.
            return Ok(false);
        }
        let get_database = (*access)
            .get_database
            .ok_or("get_database function pointer is null in duckdb_extension_access")?;
        let db_ptr = get_database(info);
        if db_ptr.is_null() {
            return Ok(false);
        }
        let db: ffi::duckdb_database = *db_ptr;

        let con = Connection::open_from_raw(db.cast())?;
        let mut raw: ffi::duckdb_connection = ptr::null_mut();
        if ffi::duckdb_connect(db, &mut raw) != ffi::duckdb_state_DuckDBSuccess {
            return Err("could not open a connection to register functions".into());
        }
        let result = register(&con, raw);
        ffi::duckdb_disconnect(&mut raw);
        result.map(|()| true)
    }
}

/// The entrypoint DuckDB calls on `LOAD duckers`.
///
/// Hand-written instead of `#[duckdb_entrypoint_c_api]`, which only exposes a duckdb-rs
/// `Connection` and not the raw connection that type and cast registration need.
///
/// # Safety
/// Called by DuckDB with valid `info` and `access` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn duckers_init_c_api(
    info: ffi::duckdb_extension_info,
    access: *const ffi::duckdb_extension_access,
) -> bool {
    unsafe {
        match init(info, access) {
            Ok(loaded) => loaded,
            Err(e) => {
                if let Some(set_error) = (*access).set_error {
                    let message = CString::new(e.to_string().replace('\0', " "))
                        .unwrap_or_else(|_| c"duckers failed to load".into());
                    set_error(info, message.as_ptr());
                }
                false
            }
        }
    }
}
