//! Copy functions: the `COPY (query) TO 'path' (FORMAT name)` side.
//!
//! The v2 C API drives a copy function through batches: bind once per statement, init once per
//! output file (one for a plain `COPY`, one per partition with `PARTITION_BY`), then for each
//! batch of rows a batch callback that prepares it and a flush callback that hands it to a file,
//! and finally finalize per file.
//!
//! Which file a batch goes to is decided only at flush. Batch callbacks run on any thread,
//! concurrently, and the init data DuckDB hands them belongs to whichever file it opened first,
//! not to the batch's file. So [`CopyTo::sink`] reads a batch's rows into a value that knows
//! nothing about files, and [`CopyTo::flush`] hands that value to its file. DuckDB serializes the
//! flushes to one file.
//!
//! The C API hands a copy function no file-system handle: the file path is a string, and the
//! implementation writes the file itself.

use std::ffi::c_void;
use std::ptr;

use plume_sys::{self as sys, ffi};

use super::error::{Error, Result, check, guard};
use super::function::opaque;
use super::types::{Context, LogicalType, Value};
use super::vector::InputVector;

/// A copy function's callbacks. The value itself is the function's user data, shared by every
/// statement that uses the format.
pub trait CopyTo: Send + Sync + 'static {
    /// Per-statement data from [`bind`](Self::bind): the resolved options.
    type BindData: Send + Sync + 'static;
    /// Per-file state from [`init`](Self::init), which [`flush`](Self::flush) changes through
    /// interior mutability.
    type File: Send + Sync + 'static;
    /// The rows of one batch, as [`sink`](Self::sink) reads them.
    type Batch: Default + Send + Sync + 'static;

    /// Checks the columns and the options of the statement.
    fn bind(&self, bind: &CopyBind<'_>) -> Result<Self::BindData>;

    /// Sets up the state for the file at `path`. With `PARTITION_BY` this runs once per partition,
    /// with the partition's file path.
    fn init(&self, bind: &Self::BindData, path: &str) -> Result<Self::File>;

    /// Reads one chunk of a batch's rows into `batch`. Batches are prepared on several threads at
    /// once, before it is known which file they go to.
    fn sink(
        &self,
        bind: &Self::BindData,
        batch: &mut Self::Batch,
        chunk: &CopyChunk<'_>,
    ) -> Result<()>;

    /// Hands a prepared batch to the file it belongs to. Runs once per batch.
    fn flush(&self, bind: &Self::BindData, file: &Self::File, batch: &Self::Batch) -> Result<()>;

    /// Runs once per file after all its batches have been flushed.
    fn finalize(&self, bind: &Self::BindData, file: &Self::File) -> Result<()>;
}

/// The statement being bound: its target path, the columns to write, and the options that DuckDB
/// did not consume itself.
///
/// DuckDB handles the generic options (`PARTITION_BY`, `OVERWRITE`, `FILENAME_PATTERN`,
/// `FILE_EXTENSION`, `USE_TMP_FILE`, ...) and leaves them out; the partition columns are not among
/// the columns either, unless `WRITE_PARTITION_COLUMNS` is set.
pub struct CopyBind<'a> {
    info: sys::duckdb_v2_copy_to_bind_info_handle,
    ctx: Context<'a>,
}

impl<'a> CopyBind<'a> {
    pub fn context(&self) -> Context<'a> {
        self.ctx
    }

    /// The path after `TO`: a file, or with `PARTITION_BY` the directory of the partitions.
    pub fn file_path(&self) -> Result<String> {
        let mut path = sys::duckdb_v2_str::default();
        check(|e| unsafe {
            ffi!(duckdb_v2_copy_to_bind_get_file_path(
                self.info, &mut path, e
            ))
        })?;
        // SAFETY: borrowed from the bind info for this callback; copied here.
        Ok(String::from_utf8_lossy(unsafe { sys::str_bytes(path) }).into_owned())
    }

    pub fn column_count(&self) -> Result<usize> {
        let mut n: sys::idx_t = 0;
        check(|e| unsafe {
            ffi!(duckdb_v2_copy_to_bind_get_column_count(
                self.info, &mut n, e
            ))
        })?;
        Ok(n as usize)
    }

    pub fn column_name(&self, index: usize) -> Result<String> {
        let mut name = sys::duckdb_v2_identifier_t::default();
        check(|e| unsafe {
            ffi!(duckdb_v2_copy_to_bind_get_column_name(
                self.info,
                index as sys::idx_t,
                &mut name,
                e
            ))
        })?;
        // SAFETY: borrowed from the bind info for this callback; copied here.
        Ok(String::from_utf8_lossy(unsafe { sys::str_bytes(name) }).into_owned())
    }

    pub fn column_type(&self, index: usize) -> Result<LogicalType> {
        let mut ty = ptr::null_mut();
        check(|e| unsafe {
            ffi!(duckdb_v2_copy_to_bind_get_column_type(
                self.info,
                index as sys::idx_t,
                &mut ty,
                e
            ))
        })?;
        // SAFETY: the getter returns an owned handle.
        Ok(unsafe { LogicalType::from_raw(ty) })
    }

    /// The remaining options as `(name, value)`, sorted by name. A bare option (`HEADER`) reads
    /// as `true`, a list of values as an unnamed struct.
    pub fn options(&self) -> Result<Vec<(String, Value)>> {
        let mut n: sys::idx_t = 0;
        check(|e| unsafe {
            ffi!(duckdb_v2_copy_to_bind_get_option_count(
                self.info, &mut n, e
            ))
        })?;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let mut name = sys::duckdb_v2_identifier_t::default();
            check(|e| unsafe {
                ffi!(duckdb_v2_copy_to_bind_get_option_name(
                    self.info, i, &mut name, e
                ))
            })?;
            // SAFETY: borrowed from the bind info for this callback; copied here.
            let name = String::from_utf8_lossy(unsafe { sys::str_bytes(name) }).into_owned();
            let mut value = ptr::null_mut();
            check(|e| unsafe {
                ffi!(duckdb_v2_copy_to_bind_get_option_value(
                    self.info, i, &mut value, e
                ))
            })?;
            // SAFETY: the getter returns an owned handle.
            out.push((name, unsafe { Value::from_raw(value) }));
        }
        Ok(out)
    }
}

/// One chunk of a batch: one vector per column, `rows` rows each.
pub struct CopyChunk<'a> {
    columns: Vec<InputVector<'a>>,
    rows: usize,
}

impl<'a> CopyChunk<'a> {
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn column(&self, index: usize) -> &InputVector<'a> {
        &self.columns[index]
    }
}

/// The bind data the wrapper keeps: the implementation's, plus the column types, which the batch
/// callback needs to create the chunk it scans into.
struct Bound<C: CopyTo> {
    data: C::BindData,
    types: Vec<LogicalType>,
}

pub(crate) fn register<C: CopyTo>(
    extension: sys::duckdb_v2_extension_handle,
    name: &str,
    f: C,
) -> Result<()> {
    let mut handle = ptr::null_mut();
    check(|err| unsafe {
        ffi!(duckdb_v2_copy_function_create_with_extension(
            extension,
            &mut handle,
            err
        ))
    })?;
    let result = (|| {
        let mut n = sys::str_view(name.as_bytes());
        check(|err| unsafe { ffi!(duckdb_v2_copy_function_set_name(handle, &mut n, err)) })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_copy_to_set_bind_callback(
                handle,
                Some(copy_bind::<C>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_copy_to_set_init_callback(
                handle,
                Some(copy_init::<C>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_copy_to_set_batch_callback(
                handle,
                Some(copy_batch::<C>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_copy_to_set_flush_callback(
                handle,
                Some(copy_flush::<C>),
                err
            ))
        })?;
        check(|err| unsafe {
            ffi!(duckdb_v2_copy_to_set_finalize_callback(
                handle,
                Some(copy_finalize::<C>),
                err
            ))
        })?;
        let mut data = opaque(f);
        check(|err| unsafe {
            ffi!(duckdb_v2_copy_function_set_user_data(
                handle, &mut data, err
            ))
        })?;
        check(|err| unsafe { ffi!(duckdb_v2_copy_function_register(handle, err)) })
    })();
    // SAFETY: the handle is ours; destroying it leaves the registered function intact.
    unsafe { ffi!(duckdb_v2_copy_function_destroy(&mut handle)) };
    result
}

/// Reads a pointer DuckDB hands back to a callback and borrows it as `T`.
///
/// # Safety
///
/// `ptr` must be null or point to a live `T` stored by this module.
unsafe fn borrow<'a, T>(ptr: *mut c_void, what: &str) -> Result<&'a T> {
    if ptr.is_null() {
        return Err(Error::internal(format!("copy function: {what} missing")));
    }
    // SAFETY: guaranteed by the caller.
    Ok(unsafe { &*ptr.cast::<T>() })
}

unsafe extern "C" fn copy_bind<C: CopyTo>(
    info: sys::duckdb_v2_copy_to_bind_info_handle,
    context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live bind info, context and error slot for this call.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_bind_get_user_data(info, &mut user, e)))?;
            let f = borrow::<C>(user, "user data")?;
            let b = CopyBind {
                info,
                ctx: Context::from_raw(context),
            };
            let data = f.bind(&b)?;
            let types = (0..b.column_count()?)
                .map(|i| b.column_type(i))
                .collect::<Result<Vec<_>>>()?;
            let mut o = opaque(Bound::<C> { data, types });
            check(|e| ffi!(duckdb_v2_copy_to_bind_set_bind_data(info, &mut o, e)))
        })
    }
}

unsafe extern "C" fn copy_init<C: CopyTo>(
    info: sys::duckdb_v2_copy_to_init_info_handle,
    _context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live init info and error slot; user and bind data are the ones
    // stored at registration and in copy_bind.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_init_get_user_data(info, &mut user, e)))?;
            let f = borrow::<C>(user, "user data")?;
            let mut b: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_init_get_bind_data(info, &mut b, e)))?;
            let bound = borrow::<Bound<C>>(b, "bind data")?;
            let mut path = sys::duckdb_v2_str::default();
            check(|e| ffi!(duckdb_v2_copy_to_init_get_file_path(info, &mut path, e)))?;
            let path = String::from_utf8_lossy(sys::str_bytes(path)).into_owned();
            let file = f.init(&bound.data, &path)?;
            let mut o = opaque(file);
            check(|e| ffi!(duckdb_v2_copy_to_init_set_init_data(info, &mut o, e)))
        })
    }
}

/// Owned handles of one batch scan, destroyed in reverse order of creation.
struct Scan {
    collection: sys::duckdb_v2_column_data_collection_handle,
    shared: sys::duckdb_v2_column_data_collection_shared_scan_state_handle,
    worker: sys::duckdb_v2_column_data_collection_worker_scan_state_handle,
    chunk: sys::duckdb_v2_data_chunk_handle,
}

impl Drop for Scan {
    fn drop(&mut self) {
        // SAFETY: every handle is owned by the scan and null or live; the destroy calls are
        // null-safe.
        unsafe {
            ffi!(duckdb_v2_data_chunk_destroy(&mut self.chunk));
            ffi!(duckdb_v2_column_data_collection_worker_scan_state_destroy(
                &mut self.worker
            ));
            ffi!(duckdb_v2_column_data_collection_shared_scan_state_destroy(
                &mut self.shared
            ));
            ffi!(duckdb_v2_column_data_collection_destroy(
                &mut self.collection
            ));
        }
    }
}

unsafe extern "C" fn copy_batch<C: CopyTo>(
    info: sys::duckdb_v2_copy_to_batch_info_handle,
    context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live batch info, context and error slot; the batch collection is
    // taken into `Scan`, which owns it and the scan states until they are dropped.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_batch_get_user_data(info, &mut user, e)))?;
            let f = borrow::<C>(user, "user data")?;
            let mut b: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_batch_get_bind_data(info, &mut b, e)))?;
            let bound = borrow::<Bound<C>>(b, "bind data")?;

            let mut batch = C::Batch::default();
            let mut scan = Scan {
                collection: ptr::null_mut(),
                shared: ptr::null_mut(),
                worker: ptr::null_mut(),
                chunk: ptr::null_mut(),
            };
            check(|e| {
                ffi!(duckdb_v2_copy_to_batch_take_input(
                    info,
                    &mut scan.collection,
                    e
                ))
            })?;
            check(|e| {
                ffi!(duckdb_v2_column_data_collection_shared_scan_state_create(
                    scan.collection,
                    &mut scan.shared,
                    e
                ))
            })?;
            check(|e| {
                ffi!(duckdb_v2_column_data_collection_worker_scan_state_create(
                    scan.collection,
                    &mut scan.worker,
                    e
                ))
            })?;
            let types: Vec<_> = bound.types.iter().map(LogicalType::raw).collect();
            check(|e| {
                ffi!(duckdb_v2_data_chunk_create_with_context(
                    context,
                    types.as_ptr(),
                    types.len() as sys::idx_t,
                    &mut scan.chunk,
                    e
                ))
            })?;
            loop {
                let mut produced = false;
                check(|e| {
                    ffi!(duckdb_v2_column_data_collection_scan(
                        scan.collection,
                        scan.shared,
                        scan.worker,
                        scan.chunk,
                        &mut produced,
                        e
                    ))
                })?;
                if !produced {
                    break;
                }
                let mut rows: sys::idx_t = 0;
                check(|e| ffi!(duckdb_v2_data_chunk_get_size(scan.chunk, &mut rows, e)))?;
                let mut columns = Vec::with_capacity(types.len());
                for c in 0..types.len() {
                    let mut v = ptr::null_mut();
                    check(|e| {
                        ffi!(duckdb_v2_data_chunk_get_vector(
                            scan.chunk,
                            c as sys::idx_t,
                            &mut v,
                            e
                        ))
                    })?;
                    columns.push(InputVector::from_raw(v, rows as usize)?);
                }
                let chunk = CopyChunk {
                    columns,
                    rows: rows as usize,
                };
                f.sink(&bound.data, &mut batch, &chunk)?;
            }
            let mut o = opaque(batch);
            check(|e| ffi!(duckdb_v2_copy_to_batch_set_batch_data(info, &mut o, e)))
        })
    }
}

unsafe extern "C" fn copy_flush<C: CopyTo>(
    info: sys::duckdb_v2_copy_to_flush_info_handle,
    _context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live flush info and error slot; user, bind, init and batch data are
    // the ones this module stored, and the batch stays alive until the flush returns.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_flush_get_user_data(info, &mut user, e)))?;
            let f = borrow::<C>(user, "user data")?;
            let mut b: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_flush_get_bind_data(info, &mut b, e)))?;
            let bound = borrow::<Bound<C>>(b, "bind data")?;
            let mut i: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_flush_get_init_data(info, &mut i, e)))?;
            let file = borrow::<C::File>(i, "init data")?;
            let mut d: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_flush_get_batch_data(info, &mut d, e)))?;
            let batch = borrow::<C::Batch>(d, "batch data")?;
            f.flush(&bound.data, file, batch)
        })
    }
}

unsafe extern "C" fn copy_finalize<C: CopyTo>(
    info: sys::duckdb_v2_copy_to_finalize_info_handle,
    _context: sys::duckdb_v2_context_handle,
    err: *mut sys::duckdb_v2_error_info_handle,
) {
    // SAFETY: DuckDB passes a live finalize info and error slot; user, bind and init data are the
    // ones this module stored.
    unsafe {
        guard(err, || {
            let mut user: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_finalize_get_user_data(info, &mut user, e)))?;
            let f = borrow::<C>(user, "user data")?;
            let mut b: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_finalize_get_bind_data(info, &mut b, e)))?;
            let bound = borrow::<Bound<C>>(b, "bind data")?;
            let mut i: *mut c_void = ptr::null_mut();
            check(|e| ffi!(duckdb_v2_copy_to_finalize_get_init_data(info, &mut i, e)))?;
            let file = borrow::<C::File>(i, "init data")?;
            f.finalize(&bound.data, file)
        })
    }
}
