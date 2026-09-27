//! Scalar functions.

use std::any::Any;
use std::ffi::c_void;
use std::ptr;

use duckers_sys::{self as sys, ffi};

use super::error::{Result, check, guard};
use super::function::{Bind, BindData, Param, configure_signature, opaque};
use super::types::{Context, LogicalType, Value};
use super::vector::{InputVector, OutputVector};

pub type ScalarExec = dyn Fn(&ScalarInput<'_>, &mut OutputVector<'_>) -> Result<()> + Send + Sync;
pub type ScalarBind = dyn Fn(&mut Bind<'_>) -> Result<Option<BindData>> + Send + Sync;

/// The arguments of one scalar exec call: one vector per parameter, `rows` rows each.
pub struct ScalarInput<'a> {
    args: Vec<InputVector<'a>>,
    rows: usize,
    bind_data: Option<&'a (dyn Any + Send + Sync)>,
    ctx: Context<'a>,
}

impl<'a> ScalarInput<'a> {
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn arg_count(&self) -> usize {
        self.args.len()
    }

    pub fn arg(&self, index: usize) -> &InputVector<'a> {
        &self.args[index]
    }

    /// Whether every argument is non-NULL at `row`.
    pub fn all_valid(&self, row: usize) -> bool {
        self.args.iter().all(|a| a.is_valid(row))
    }

    /// The bind data the bind callback returned, if it has type `T`.
    pub fn bind_data<T: 'static>(&self) -> Option<&T> {
        self.bind_data.and_then(|d| d.downcast_ref::<T>())
    }

    pub fn context(&self) -> Context<'a> {
        self.ctx
    }
}

/// A value a row-wise scalar function writes into its result vector.
pub trait WriteCell {
    fn write(self, out: &mut OutputVector<'_>, row: usize) -> Result<()>;
}

impl WriteCell for String {
    fn write(self, out: &mut OutputVector<'_>, row: usize) -> Result<()> {
        out.set_str(row, &self)
    }
}

impl WriteCell for &str {
    fn write(self, out: &mut OutputVector<'_>, row: usize) -> Result<()> {
        out.set_str(row, self)
    }
}

impl WriteCell for Vec<u8> {
    fn write(self, out: &mut OutputVector<'_>, row: usize) -> Result<()> {
        out.set_bytes(row, &self)
    }
}

impl WriteCell for &[u8] {
    fn write(self, out: &mut OutputVector<'_>, row: usize) -> Result<()> {
        out.set_bytes(row, self)
    }
}

macro_rules! write_cell_fixed {
    ($($t:ty),*) => {$(
        impl WriteCell for $t {
            fn write(self, out: &mut OutputVector<'_>, row: usize) -> Result<()> {
                // SAFETY: the function's declared return type has this physical layout; that is
                // the contract of `ScalarFunction::map_rows`.
                unsafe { out.set(row, self) };
                Ok(())
            }
        }
    )*};
}
write_cell_fixed!(bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);

/// A scalar function under construction; register it with
/// [`Extension::register_scalar`](super::Extension::register_scalar).
pub struct ScalarFunction {
    pub(crate) name: String,
    pub(crate) params: Vec<Param>,
    pub(crate) varargs: Option<LogicalType>,
    pub(crate) returns: LogicalType,
    pub(crate) bind: Option<Box<ScalarBind>>,
    pub(crate) exec: Box<ScalarExec>,
    pub(crate) volatile: bool,
    pub(crate) special_nulls: bool,
}

impl ScalarFunction {
    /// A function with a vectorised exec callback, which writes all `input.rows()` rows.
    pub fn new(
        name: impl Into<String>,
        returns: &LogicalType,
        exec: impl Fn(&ScalarInput<'_>, &mut OutputVector<'_>) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            params: Vec::new(),
            varargs: None,
            returns: returns.clone(),
            bind: None,
            exec: Box::new(exec),
            volatile: false,
            special_nulls: false,
        }
    }

    /// A function computed row by row. A row with a NULL argument yields NULL without calling `f`
    /// (NULL in, NULL out), and `Ok(None)` from `f` also writes NULL. The result type `R` must
    /// have the physical layout of `returns` (`String` for `VARCHAR`, `Vec<u8>` for `BLOB` and
    /// the custom types, `i64` for `BIGINT`, ...).
    pub fn map_rows<R: WriteCell>(
        name: impl Into<String>,
        returns: &LogicalType,
        f: impl Fn(&ScalarInput<'_>, usize) -> Result<Option<R>> + Send + Sync + 'static,
    ) -> Self {
        Self::new(name, returns, move |input, out| {
            for row in 0..input.rows() {
                if !input.all_valid(row) {
                    out.set_null(row)?;
                    continue;
                }
                match f(input, row)? {
                    Some(v) => v.write(out, row)?,
                    None => out.set_null(row)?,
                }
            }
            Ok(())
        })
    }

    /// A positional parameter.
    pub fn param(mut self, name: &str, ty: &LogicalType) -> Self {
        self.params.push(Param::new(name, ty));
        self
    }

    /// An optional parameter with a default, passable by name (`name := value`).
    pub fn named(mut self, name: &str, ty: &LogicalType, default: Value) -> Self {
        self.params.push(Param::with_default(name, ty, default));
        self
    }

    pub fn varargs(mut self, ty: &LogicalType) -> Self {
        self.varargs = Some(ty.clone());
        self
    }

    /// A bind callback, run once per call site at plan time. It can check argument types, set
    /// the return type, and return bind data for [`ScalarInput::bind_data`].
    pub fn bind(
        mut self,
        f: impl Fn(&mut Bind<'_>) -> Result<Option<BindData>> + Send + Sync + 'static,
    ) -> Self {
        self.bind = Some(Box::new(f));
        self
    }

    /// Marks the function volatile: never folded at plan time, evaluated for every row.
    pub fn volatile(mut self) -> Self {
        self.volatile = true;
        self
    }

    /// Calls the function for rows with NULL arguments too, instead of DuckDB folding them.
    pub fn special_nulls(mut self) -> Self {
        self.special_nulls = true;
        self
    }
}

struct ScalarImpl {
    name: String,
    bind: Option<Box<ScalarBind>>,
    exec: Box<ScalarExec>,
}

pub(crate) fn register(
    extension: sys::duckdb_v2_extension_handle,
    f: ScalarFunction,
) -> Result<()> {
    let mut handle = ptr::null_mut();
    check(|err| unsafe {
        ffi!(duckdb_v2_scalar_function_create_with_extension(
            extension,
            &mut handle,
            err
        ))
    })?;
    let result = (|| {
        let mut name = sys::str_view(f.name.as_bytes());
        check(|err| unsafe { ffi!(duckdb_v2_scalar_function_set_name(handle, &mut name, err)) })?;
        let mut sig = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_scalar_function_get_signature(
                handle, &mut sig, err
            ))
        })?;
        configure_signature(sig, &f.params, f.varargs.as_ref(), &f.returns)?;
        if f.volatile {
            check(|err| unsafe {
                ffi!(duckdb_v2_scalar_function_set_property(
                    handle,
                    sys::DUCKDB_V2_FUNCTION_PROPERTY_STABILITY,
                    sys::DUCKDB_V2_FUNCTION_PROPERTY_STABILITY_VOLATILE,
                    err
                ))
            })?;
        }
        if f.special_nulls {
            check(|err| unsafe {
                ffi!(duckdb_v2_scalar_function_set_property(
                    handle,
                    sys::DUCKDB_V2_FUNCTION_PROPERTY_NULL_HANDLING,
                    sys::DUCKDB_V2_FUNCTION_PROPERTY_NULL_HANDLING_SPECIAL,
                    err
                ))
            })?;
        }
        if f.bind.is_some() {
            check(|err| unsafe {
                ffi!(duckdb_v2_scalar_function_set_bind_callback(
                    handle,
                    Some(scalar_bind),
                    err
                ))
            })?;
        }
        check(|err| unsafe {
            ffi!(duckdb_v2_scalar_function_set_exec_callback(
                handle,
                Some(scalar_exec),
                err
            ))
        })?;
        let mut data = opaque(ScalarImpl {
            name: f.name.clone(),
            bind: f.bind,
            exec: f.exec,
        });
        check(|err| unsafe {
            ffi!(duckdb_v2_scalar_function_set_user_data(
                handle, &mut data, err
            ))
        })?;
        check(|err| unsafe { ffi!(duckdb_v2_scalar_function_register(handle, err)) })
    })();
    // SAFETY: the handle is ours; destroying it leaves the registered function intact.
    unsafe { ffi!(duckdb_v2_scalar_function_destroy(&mut handle)) };
    result
}

unsafe extern "C" fn scalar_bind(
    info: sys::duckdb_v2_scalar_function_bind_info_handle,
    context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live bind info, context and error slot for this call.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_scalar_function_bind_get_user_data(
                    info, &mut user, e
                ))
            })?;
            let imp = &*user.cast::<ScalarImpl>();
            let Some(bind) = &imp.bind else { return Ok(()) };
            let mut b = Bind::scalar(info, Context::from_raw(context), &imp.name);
            if let Some(data) = bind(&mut b)? {
                let mut o = opaque::<BindData>(data);
                check(|e| {
                    ffi!(duckdb_v2_scalar_function_bind_set_bind_data(
                        info, &mut o, e
                    ))
                })?;
            }
            Ok(())
        })
    }
}

unsafe extern "C" fn scalar_exec(
    info: sys::duckdb_v2_scalar_function_exec_info_handle,
    context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live exec info, context and error slot for this call; the vectors
    // it hands out stay valid until the callback returns.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_scalar_function_exec_get_user_data(
                    info, &mut user, e
                ))
            })?;
            let imp = &*user.cast::<ScalarImpl>();
            let mut bind: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_scalar_function_exec_get_bind_data(
                    info, &mut bind, e
                ))
            })?;
            let bind_data = (!bind.is_null()).then(|| &**bind.cast::<BindData>());
            let mut rows: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_scalar_function_exec_get_row_count(
                    info, &mut rows, e
                ))
            })?;
            let mut argc: u32 = 0;
            check(|e| {
                ffi!(duckdb_v2_scalar_function_exec_get_arg_count(
                    info, &mut argc, e
                ))
            })?;
            let mut args = Vec::with_capacity(argc as usize);
            for i in 0..argc {
                let mut v = ptr::null_mut();
                check(|e| ffi!(duckdb_v2_scalar_function_exec_get_arg(info, i, &mut v, e)))?;
                args.push(InputVector::from_raw(v, rows as usize)?);
            }
            let mut result = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_scalar_function_exec_get_result(
                    info,
                    &mut result,
                    e
                ))
            })?;
            let mut out = OutputVector::from_raw(result)?;
            let input = ScalarInput {
                args,
                rows: rows as usize,
                bind_data,
                ctx: Context::from_raw(context),
            };
            (imp.exec)(&input, &mut out)
        })
    }
}
