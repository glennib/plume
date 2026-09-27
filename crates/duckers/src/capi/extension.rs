//! The loading extension: the entrypoint glue and the registration calls.

use std::ptr;

use duckers_sys::{self as sys, ffi};

use super::aggregate::{self, Aggregate, AggregateFunction};
use super::cast::{self, CastRow};
use super::copy::{self, CopyTo};
use super::error::{Error, Result, check};
use super::scalar::{self, ScalarFunction};
use super::types::{Context, LogicalType};
use super::vector::InputVector;

/// The extension being loaded, valid during the entrypoint. Everything duckers adds to DuckDB is
/// registered through it.
pub struct Extension<'a> {
    handle: sys::duckdb_v2_extension_handle,
    ctx: Context<'a>,
}

impl<'a> Extension<'a> {
    /// The entrypoint's context, for creating types and default values.
    pub fn context(&self) -> Context<'a> {
        self.ctx
    }

    pub fn register_scalar(&self, f: ScalarFunction) -> Result<()> {
        let name = f.name.clone();
        scalar::register(self.handle, f)
            .map_err(|e| registration_error("scalar function", &name, e))
    }

    pub fn register_aggregate<A: Aggregate>(&self, f: AggregateFunction<A>) -> Result<()> {
        let name = f.name.clone();
        aggregate::register(self.handle, f).map_err(|e| registration_error("aggregate", &name, e))
    }

    /// Registers a copy function: the format `name` of `COPY ... TO ... (FORMAT name)`.
    pub fn register_copy<C: CopyTo>(&self, name: &str, f: C) -> Result<()> {
        copy::register(self.handle, name, f)
            .map_err(|e| registration_error("copy function", name, e))
    }

    /// Registers a custom type `name` over `base`, and returns it as a logical type.
    pub fn register_type(&self, name: &str, base: &LogicalType) -> Result<LogicalType> {
        let mut handle = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_custom_type_create_with_extension(
                self.handle,
                &mut handle,
                err
            ))
        })?;
        let result = (|| {
            check(|err| unsafe {
                ffi!(duckdb_v2_custom_type_set_name(
                    handle,
                    sys::str_view(name.as_bytes()),
                    err
                ))
            })?;
            check(|err| unsafe {
                ffi!(duckdb_v2_custom_type_set_base_type(handle, base.raw(), err))
            })?;
            check(|err| unsafe { ffi!(duckdb_v2_custom_type_register(handle, err)) })
        })();
        // SAFETY: the handle is ours; destroying it leaves the registered type intact.
        unsafe { ffi!(duckdb_v2_custom_type_destroy(&mut handle)) };
        result.map_err(|e| registration_error("type", name, e))?;
        self.ctx.type_from_text(name)
    }

    /// Registers a cast whose result is written as bytes: see
    /// [`register_bytes_cast`](super::cast::register_bytes_cast).
    pub fn register_bytes_cast(
        &self,
        source: &LogicalType,
        target: &LogicalType,
        f: impl Fn(&InputVector<'_>, usize) -> Result<Option<Vec<u8>>> + Send + Sync + 'static,
    ) -> Result<()> {
        let f: Box<CastRow> = Box::new(f);
        cast::register_bytes_cast(self.handle, source, target, f).map_err(|e| {
            registration_error(
                "cast",
                &format!("{} -> {}", source.to_text(), target.to_text()),
                e,
            )
        })
    }
}

fn registration_error(what: &str, name: &str, e: Error) -> Error {
    Error::new(e.code(), format!("duckers: registering {what} {name}: {e}"))
}

/// Runs an extension's load function from its `<name>_init_c_api_v2` entrypoint.
///
/// # Safety
///
/// `input` must be the pointer DuckDB passed to the entrypoint.
pub unsafe fn entrypoint(
    input: *mut sys::duckdb_v2_extension_input,
    load: impl FnOnce(&Extension<'_>) -> Result<()>,
) {
    // SAFETY: forwarded from the caller.
    if !unsafe { sys::init_api(input) } {
        // The loader has recorded why; the error slot must stay untouched.
        return;
    }
    // SAFETY: the input fields are valid for the duration of the entrypoint.
    let (handle, ctx, err) = unsafe { ((*input).extension, (*input).context, (*input).err) };
    let ext = Extension {
        handle,
        // SAFETY: valid until the entrypoint returns, which outlives `ext`.
        ctx: unsafe { Context::from_raw(ctx) },
    };
    // SAFETY: `err` is the loader's live error slot.
    unsafe { super::error::guard(err, || load(&ext)) };
}
