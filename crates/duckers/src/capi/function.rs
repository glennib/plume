//! Pieces shared by the function families: parameters, bind info and bind data.

use std::any::Any;
use std::ffi::c_void;
use std::ptr;

use duckers_sys::{self as sys, ffi};

use super::error::{Error, Result, check};
use super::types::{Context, LogicalType, TypeId, Value};

/// One parameter of a function signature. A parameter with a default is optional and can be
/// passed by name (`f(x, sep := '|')`); DuckDB fills the default in at bind time, so bind and
/// exec see every parameter as a positional argument in declaration order.
pub struct Param {
    pub(crate) name: String,
    pub(crate) ty: LogicalType,
    pub(crate) default: Option<Value>,
}

impl Param {
    pub fn new(name: impl Into<String>, ty: &LogicalType) -> Self {
        Self {
            name: name.into(),
            ty: ty.clone(),
            default: None,
        }
    }

    pub fn with_default(name: impl Into<String>, ty: &LogicalType, default: Value) -> Self {
        Self {
            name: name.into(),
            ty: ty.clone(),
            default: Some(default),
        }
    }
}

/// Adds the parameters, varargs and return type to a signature handle.
pub(crate) fn configure_signature(
    sig: sys::duckdb_v2_function_signature_handle,
    params: &[Param],
    varargs: Option<&LogicalType>,
    returns: &LogicalType,
) -> Result<()> {
    for p in params {
        let default = p.default.as_ref().map_or(ptr::null_mut(), Value::raw);
        check(|err| unsafe {
            ffi!(duckdb_v2_function_signature_add_parameter(
                sig,
                sys::str_view(p.name.as_bytes()),
                p.ty.raw(),
                default,
                err
            ))
        })?;
    }
    if let Some(v) = varargs {
        check(|err| unsafe { ffi!(duckdb_v2_function_signature_set_varargs(sig, v.raw(), err)) })?;
    }
    check(|err| unsafe {
        ffi!(duckdb_v2_function_signature_set_return_type(
            sig,
            returns.raw(),
            err
        ))
    })
}

/// Bind data attached to one call site, handed to later callbacks of the same call.
pub type BindData = Box<dyn Any + Send + Sync>;

#[derive(Clone, Copy)]
enum BindHandle {
    Scalar(sys::duckdb_v2_scalar_function_bind_info_handle),
    Aggregate(sys::duckdb_v2_aggregate_function_bind_info_handle),
}

/// The call site being bound: argument types, constant argument values, and the return type.
pub struct Bind<'a> {
    handle: BindHandle,
    ctx: Context<'a>,
    name: &'a str,
}

impl<'a> Bind<'a> {
    pub(crate) fn scalar(
        handle: sys::duckdb_v2_scalar_function_bind_info_handle,
        ctx: Context<'a>,
        name: &'a str,
    ) -> Self {
        Self {
            handle: BindHandle::Scalar(handle),
            ctx,
            name,
        }
    }

    pub(crate) fn aggregate(
        handle: sys::duckdb_v2_aggregate_function_bind_info_handle,
        ctx: Context<'a>,
        name: &'a str,
    ) -> Self {
        Self {
            handle: BindHandle::Aggregate(handle),
            ctx,
            name,
        }
    }

    pub fn context(&self) -> Context<'a> {
        self.ctx
    }

    /// The function name, for error messages.
    pub fn function_name(&self) -> &str {
        self.name
    }

    pub fn arg_count(&self) -> usize {
        let mut n: sys::idx_t = 0;
        let _ = match self.handle {
            BindHandle::Scalar(h) => check(|err| unsafe {
                ffi!(duckdb_v2_scalar_function_bind_get_arg_count(h, &mut n, err))
            }),
            BindHandle::Aggregate(h) => check(|err| unsafe {
                ffi!(duckdb_v2_aggregate_function_bind_get_arg_count(
                    h, &mut n, err
                ))
            }),
        };
        n as usize
    }

    /// The concrete type of argument `index`, also for `ANY` parameters.
    pub fn arg_type(&self, index: usize) -> Result<LogicalType> {
        let mut out = ptr::null_mut();
        let i = index as sys::idx_t;
        match self.handle {
            BindHandle::Scalar(h) => check(|err| unsafe {
                ffi!(duckdb_v2_scalar_function_bind_get_arg_type(
                    h, i, &mut out, err
                ))
            })?,
            BindHandle::Aggregate(h) => check(|err| unsafe {
                ffi!(duckdb_v2_aggregate_function_bind_get_arg_type(
                    h, i, &mut out, err
                ))
            })?,
        }
        // SAFETY: the getter returns an owned handle.
        Ok(unsafe { LogicalType::from_raw(out) })
    }

    /// Folds argument `index` to a constant. Fails for arguments that are not constant.
    pub fn arg_value(&self, index: usize) -> Result<Value> {
        let mut out = ptr::null_mut();
        let i = index as sys::idx_t;
        match self.handle {
            BindHandle::Scalar(h) => check(|err| unsafe {
                ffi!(duckdb_v2_scalar_function_bind_get_arg_value(
                    h, i, &mut out, err
                ))
            })?,
            BindHandle::Aggregate(h) => check(|err| unsafe {
                ffi!(duckdb_v2_aggregate_function_bind_get_arg_value(
                    h, i, &mut out, err
                ))
            })?,
        }
        // SAFETY: the getter returns an owned handle.
        Ok(unsafe { Value::from_raw(out) })
    }

    /// Like [`arg_value`](Self::arg_value), with an error naming the parameter when the
    /// argument is not a constant.
    pub fn constant_arg(&self, index: usize, param: &str) -> Result<Value> {
        self.arg_value(index).map_err(|_| {
            Error::binder(format!(
                "{}: the argument {param} must be a constant",
                self.name
            ))
        })
    }

    pub fn set_return_type(&self, ty: &LogicalType) -> Result<()> {
        match self.handle {
            BindHandle::Scalar(h) => check(|err| unsafe {
                ffi!(duckdb_v2_scalar_function_bind_set_return_type(
                    h,
                    ty.raw(),
                    err
                ))
            }),
            BindHandle::Aggregate(h) => check(|err| unsafe {
                ffi!(duckdb_v2_aggregate_function_bind_set_return_type(
                    h,
                    ty.raw(),
                    err
                ))
            }),
        }
    }

    /// Checks that argument `index` has one of the accepted type ids and returns its type.
    /// The error reads like DuckDB's own for a typed overload that does not match.
    pub fn expect_arg(
        &self,
        index: usize,
        param: &str,
        accepted: &[TypeId],
        what: &str,
    ) -> Result<LogicalType> {
        let ty = self.arg_type(index)?;
        if accepted.contains(&ty.id()) {
            Ok(ty)
        } else {
            Err(Error::binder(format!(
                "{}: {param} must be {what}, got {}",
                self.name,
                ty.to_text()
            )))
        }
    }
}

/// `duckdb_v2_opaque` destroy callback for a `Box<T>`.
pub(crate) unsafe extern "C" fn drop_box<T>(data: *mut c_void) {
    if !data.is_null() {
        // SAFETY: `data` came from `Box::<T>::into_raw` and is destroyed once.
        drop(unsafe { Box::from_raw(data.cast::<T>()) });
    }
}

/// Packs a value into a `duckdb_v2_opaque` that DuckDB owns and destroys.
pub(crate) fn opaque<T>(value: T) -> sys::duckdb_v2_opaque {
    sys::duckdb_v2_opaque {
        ptr: Box::into_raw(Box::new(value)).cast(),
        destroy: Some(drop_box::<T>),
        equals: None,
    }
}
