//! Aggregate functions.
//!
//! The v2 C API allows one aggregate per name (a second registration under the same name fails),
//! so an aggregate that accepts several argument types declares its parameters `ANY` and checks
//! the concrete types in [`Aggregate::bind`]. Every aggregate is registered order-independent:
//! `agg(x ORDER BY y)` and `agg(x) OVER ()` crash DuckDB for order-dependent C-API aggregates
//! (duckdb#26109), and with the property set the planner drops the `ORDER BY` instead.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;

use plume_sys::{self as sys, ffi};

use super::error::{Result, check, guard};
use super::function::{Bind, Param, configure_signature, opaque};
use super::types::{Context, LogicalType, Value};
use super::vector::{InputVector, OutputVector};

/// The arguments of one update call: one vector per parameter.
pub struct AggregateInput<'a> {
    args: Vec<InputVector<'a>>,
}

impl<'a> AggregateInput<'a> {
    pub fn arg(&self, index: usize) -> &InputVector<'a> {
        &self.args[index]
    }

    pub fn arg_count(&self) -> usize {
        self.args.len()
    }

    pub fn all_valid(&self, row: usize) -> bool {
        self.args.iter().all(|a| a.is_valid(row))
    }
}

/// An aggregate's state and callbacks. One value of `Self` is the state of one group.
///
/// States live in DuckDB-allocated memory as a pointer to a boxed `Self`; the wrapper allocates it
/// on first use and frees it in the destroy callback, so implementations are plain Rust.
pub trait Aggregate: Sized + Send + 'static {
    /// Per-call-site data computed in [`bind`](Self::bind): resolved argument kinds, constant
    /// parameter values.
    type BindData: Send + Sync + 'static;

    /// Checks the argument types, sets the return type if it is `ANY`, and computes bind data.
    fn bind(bind: &mut Bind<'_>) -> Result<Self::BindData>;

    /// A fresh, empty state.
    fn init(bind: &Self::BindData) -> Self;

    /// Adds input row `row` to this state. NULL arguments reach this call; skip them here.
    fn update(
        &mut self,
        bind: &Self::BindData,
        input: &AggregateInput<'_>,
        row: usize,
    ) -> Result<()>;

    /// Merges `other` into this state; `other` must stay unchanged.
    fn combine(&mut self, bind: &Self::BindData, other: &Self) -> Result<()>;

    /// Writes the state's result into `out` at `row`.
    fn finalize(
        &mut self,
        bind: &Self::BindData,
        out: &mut OutputVector<'_>,
        row: usize,
    ) -> Result<()>;
}

/// An aggregate function under construction; register it with
/// [`Extension::register_aggregate`](super::Extension::register_aggregate).
pub struct AggregateFunction<A: Aggregate> {
    pub(crate) name: String,
    pub(crate) params: Vec<Param>,
    pub(crate) varargs: Option<LogicalType>,
    pub(crate) returns: LogicalType,
    _state: PhantomData<fn() -> A>,
}

impl<A: Aggregate> AggregateFunction<A> {
    /// `returns` may be `ANY`, in which case [`Aggregate::bind`] must set the concrete type.
    pub fn new(name: impl Into<String>, returns: &LogicalType) -> Self {
        Self {
            name: name.into(),
            params: Vec::new(),
            varargs: None,
            returns: returns.clone(),
            _state: PhantomData,
        }
    }

    pub fn param(mut self, name: &str, ty: &LogicalType) -> Self {
        self.params.push(Param::new(name, ty));
        self
    }

    /// An optional parameter with a default, passable by name (`name := expr`). The argument
    /// need not be constant; a non-constant one arrives as a vector like any other argument.
    pub fn named(mut self, name: &str, ty: &LogicalType, default: Value) -> Self {
        self.params.push(Param::with_default(name, ty, default));
        self
    }

    pub fn varargs(mut self, ty: &LogicalType) -> Self {
        self.varargs = Some(ty.clone());
        self
    }
}

struct AggregateImpl {
    name: String,
}

pub(crate) fn register<A: Aggregate>(
    extension: sys::duckdb_v2_extension_handle,
    f: AggregateFunction<A>,
) -> Result<()> {
    let mut handle = ptr::null_mut();
    check(|err| unsafe {
        ffi!(duckdb_v2_aggregate_function_create_with_extension(
            extension,
            &mut handle,
            err
        ))
    })?;
    let result = (|| {
        let mut name = sys::str_view(f.name.as_bytes());
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_name(
                handle, &mut name, err
            ))
        })?;
        let mut sig = ptr::null_mut();
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_get_signature(
                handle, &mut sig, err
            ))
        })?;
        configure_signature(sig, &f.params, f.varargs.as_ref(), &f.returns)?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_property(
                handle,
                sys::DUCKDB_V2_FUNCTION_PROPERTY_AGG_ORDER_DEPENDENT,
                sys::DUCKDB_V2_FUNCTION_PROPERTY_AGG_ORDER_DEPENDENT_NO,
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_bind_callback(
                handle,
                Some(agg_bind::<A>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_size_callback(
                handle,
                Some(agg_size),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_init_callback(
                handle,
                Some(agg_init),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_update_callback(
                handle,
                Some(agg_update::<A>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_combine_callback(
                handle,
                Some(agg_combine::<A>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_finalize_callback(
                handle,
                Some(agg_finalize::<A>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_destroy_callback(
                handle,
                Some(agg_destroy::<A>),
                err
            ))
        })?;
        let mut data = opaque(AggregateImpl {
            name: f.name.clone(),
        });
        check(|err| unsafe {
            ffi!(duckdb_v2_aggregate_function_set_user_data(
                handle, &mut data, err
            ))
        })?;
        check(|err| unsafe { ffi!(duckdb_v2_aggregate_function_register(handle, err)) })
    })();
    // SAFETY: the handle is ours; destroying it leaves the registered function intact.
    unsafe { ffi!(duckdb_v2_aggregate_function_destroy(&mut handle)) };
    result
}

/// A state slot: DuckDB-owned memory holding a (possibly null) pointer to a boxed state.
type Slot<A> = *mut *mut A;

unsafe fn slot_get<A>(slot: *mut c_void) -> *mut A {
    // SAFETY: the slot holds a pointer written by init or by `slot_get_or_init`.
    unsafe { ptr::read_unaligned(slot as Slot<A>) }
}

unsafe fn slot_get_or_init<A: Aggregate>(slot: *mut c_void, bind: &A::BindData) -> *mut A {
    // SAFETY: see slot_get; a null slot gets a fresh state it then owns.
    unsafe {
        let mut p = slot_get::<A>(slot);
        if p.is_null() {
            p = Box::into_raw(Box::new(A::init(bind)));
            ptr::write_unaligned(slot as Slot<A>, p);
        }
        p
    }
}

unsafe fn slice<'a, T>(ptr: *mut T, len: sys::idx_t) -> &'a [T] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: DuckDB hands out arrays of `len` entries.
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

unsafe extern "C" fn agg_bind<A: Aggregate>(
    info: sys::duckdb_v2_aggregate_function_bind_info_handle,
    context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live bind info, context and error slot for this call.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_bind_get_user_data(
                    info, &mut user, e
                ))
            })?;
            let imp = &*user.cast::<AggregateImpl>();
            let mut b = Bind::aggregate(info, Context::from_raw(context), &imp.name);
            let data = A::bind(&mut b)?;
            let mut o = opaque::<A::BindData>(data);
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_bind_set_bind_data(
                    info, &mut o, e
                ))
            })
        })
    }
}

unsafe extern "C" fn agg_size(
    info: sys::duckdb_v2_aggregate_function_size_info_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live size info and error slot.
    unsafe {
        guard(err, || {
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_size_set_state_size(
                    info,
                    size_of::<*mut c_void>() as sys::idx_t,
                    e
                ))
            })
        })
    }
}

unsafe extern "C" fn agg_init(
    info: sys::duckdb_v2_aggregate_function_init_info_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes `count` state slots of the size agg_size reported.
    unsafe {
        guard(err, || {
            let mut count: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_init_get_state_count(
                    info, &mut count, e
                ))
            })?;
            let mut states: *mut *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_init_get_states(
                    info,
                    &mut states,
                    e
                ))
            })?;
            for &s in slice(states, count) {
                ptr::write_unaligned(s.cast::<*mut c_void>(), ptr::null_mut());
            }
            Ok(())
        })
    }
}

unsafe fn bind_data<'a, T>(ptr: *mut c_void) -> Result<&'a T> {
    if ptr.is_null() {
        return Err(super::error::Error::internal("aggregate bind data missing"));
    }
    // SAFETY: agg_bind stored a `Box<T>` for this call site.
    Ok(unsafe { &*ptr.cast::<T>() })
}

unsafe extern "C" fn agg_update<A: Aggregate>(
    info: sys::duckdb_v2_aggregate_function_update_info_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes `rows` argument rows and one state slot per row; several rows may
    // share a slot, which the sequential loop handles.
    unsafe {
        guard(err, || {
            let mut b: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_update_get_bind_data(
                    info, &mut b, e
                ))
            })?;
            let bind = bind_data::<A::BindData>(b)?;
            let mut rows: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_update_get_row_count(
                    info, &mut rows, e
                ))
            })?;
            let mut argc: u32 = 0;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_update_get_arg_count(
                    info, &mut argc, e
                ))
            })?;
            let mut args = Vec::with_capacity(argc as usize);
            for i in 0..argc {
                let mut v = ptr::null_mut();
                check(|e| {
                    ffi!(duckdb_v2_aggregate_function_update_get_arg(
                        info, i, &mut v, e
                    ))
                })?;
                args.push(InputVector::from_raw(v, rows as usize)?);
            }
            let input = AggregateInput { args };
            let mut states: *mut *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_update_get_states(
                    info,
                    &mut states,
                    e
                ))
            })?;
            for (row, &slot) in slice(states, rows).iter().enumerate() {
                let state = &mut *slot_get_or_init::<A>(slot, bind);
                state.update(bind, &input, row)?;
            }
            Ok(())
        })
    }
}

unsafe extern "C" fn agg_combine<A: Aggregate>(
    info: sys::duckdb_v2_aggregate_function_combine_info_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes `count` source/target slot pairs.
    unsafe {
        guard(err, || {
            let mut b: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_combine_get_bind_data(
                    info, &mut b, e
                ))
            })?;
            let bind = bind_data::<A::BindData>(b)?;
            let mut count: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_combine_get_state_count(
                    info, &mut count, e
                ))
            })?;
            let mut sources: *mut *mut c_void = ptr::null_mut();
            let mut targets: *mut *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_combine_get_sources(
                    info,
                    &mut sources,
                    e
                ))
            })?;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_combine_get_targets(
                    info,
                    &mut targets,
                    e
                ))
            })?;
            for (&src, &dst) in slice(sources, count).iter().zip(slice(targets, count)) {
                let s = slot_get::<A>(src);
                if s.is_null() {
                    continue;
                }
                let t = &mut *slot_get_or_init::<A>(dst, bind);
                t.combine(bind, &*s)?;
            }
            Ok(())
        })
    }
}

unsafe extern "C" fn agg_finalize<A: Aggregate>(
    info: sys::duckdb_v2_aggregate_function_finalize_info_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes `count` state slots and a result vector with room for rows
    // `offset..offset + count`.
    unsafe {
        guard(err, || {
            let mut b: *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_finalize_get_bind_data(
                    info, &mut b, e
                ))
            })?;
            let bind = bind_data::<A::BindData>(b)?;
            let mut count: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_finalize_get_state_count(
                    info, &mut count, e
                ))
            })?;
            let mut offset: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_finalize_get_result_offset(
                    info,
                    &mut offset,
                    e
                ))
            })?;
            let mut states: *mut *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_finalize_get_states(
                    info,
                    &mut states,
                    e
                ))
            })?;
            let mut result = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_finalize_get_result(
                    info,
                    &mut result,
                    e
                ))
            })?;
            let mut out = OutputVector::from_raw(result)?;
            for (i, &slot) in slice(states, count).iter().enumerate() {
                // An untouched state (a group with no rows, or an empty ungrouped input) gets a
                // fresh one, so finalize always sees a state; destroy frees it.
                let state = &mut *slot_get_or_init::<A>(slot, bind);
                state.finalize(bind, &mut out, offset as usize + i)?;
            }
            Ok(())
        })
    }
}

unsafe extern "C" fn agg_destroy<A: Aggregate>(
    info: sys::duckdb_v2_aggregate_function_destroy_info_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes `count` state slots that are not used again. It destroys finalized
    // states too, so this is the only place states are freed.
    unsafe {
        guard(err, || {
            let mut count: sys::idx_t = 0;
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_destroy_get_state_count(
                    info, &mut count, e
                ))
            })?;
            let mut states: *mut *mut c_void = ptr::null_mut();
            check(|e| {
                ffi!(duckdb_v2_aggregate_function_destroy_get_states(
                    info,
                    &mut states,
                    e
                ))
            })?;
            for &slot in slice(states, count) {
                let p = slot_get::<A>(slot);
                if !p.is_null() {
                    drop(Box::from_raw(p));
                    ptr::write_unaligned(slot as Slot<A>, ptr::null_mut());
                }
            }
            Ok(())
        })
    }
}
