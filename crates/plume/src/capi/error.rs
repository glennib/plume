//! Errors crossing the C API boundary in both directions.

use std::fmt;
use std::ptr;

use plume_sys::{self as sys, ffi};

/// An error with the DuckDB error code it is reported under.
///
/// The code picks the exception DuckDB raises, and so the message prefix the user sees:
/// [`Error::binder`] becomes a "Binder Error", [`Error::invalid_input`] an "Invalid Input Error".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    code: sys::DUCKDB_V2_ERROR,
    message: String,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(code: sys::DUCKDB_V2_ERROR, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// A bind-time error: wrong argument types, non-constant arguments that must be constant.
    pub fn binder(message: impl Into<String>) -> Self {
        Self::new(sys::DUCKDB_V2_ERROR_QUERY_BINDER, message)
    }

    /// A runtime error caused by a bad value.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(sys::DUCKDB_V2_ERROR_INPUT_INVALID, message)
    }

    /// A value that cannot be converted, as raised by casts.
    pub fn conversion(message: impl Into<String>) -> Self {
        Self::new(sys::DUCKDB_V2_ERROR_TYPE_CONVERSION, message)
    }

    /// A failure to reach something outside DuckDB, such as a display.
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(sys::DUCKDB_V2_ERROR_IO_GENERAL, message)
    }

    /// A broken invariant inside plume.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(sys::DUCKDB_V2_ERROR_RUNTIME_INTERNAL, message)
    }

    pub fn code(&self) -> sys::DUCKDB_V2_ERROR {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Writes this error into a callback's error slot, which makes DuckDB raise it.
    ///
    /// # Safety
    ///
    /// `slot` must be the live error slot DuckDB passed to the running callback.
    pub(crate) unsafe fn report(&self, slot: *mut sys::duckdb_v2_error_info_handle) {
        if slot.is_null() {
            return;
        }
        // SAFETY: the slot is live for the callback; set_code and set_text accept its handle and
        // copy the text.
        unsafe {
            let info = *slot;
            ffi!(duckdb_v2_error_info_set_code(info, self.code));
            ffi!(duckdb_v2_error_info_set_text(
                info,
                sys::str_view(self.message.as_bytes())
            ));
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Runs one fallible C API call with an error slot of its own and converts a failure to [`Error`].
///
/// ```ignore
/// check(|err| unsafe { ffi!(duckdb_v2_vector_flatten(v, err)) })?;
/// ```
pub(crate) fn check(
    call: impl FnOnce(*mut sys::duckdb_v2_error_info_handle) -> sys::DUCKDB_V2_ERROR,
) -> Result<()> {
    let mut info: sys::duckdb_v2_error_info_handle = ptr::null_mut();
    let code = call(&mut info);
    if code == sys::DUCKDB_V2_ERROR_NONE {
        return Ok(());
    }
    let mut message = String::from("DuckDB C API call failed");
    if !info.is_null() {
        let mut text = sys::duckdb_v2_str::default();
        // SAFETY: info is the handle DuckDB allocated for this failure; its text is valid until
        // it is destroyed, and it is copied before that.
        unsafe {
            if ffi!(duckdb_v2_error_info_get_raw_text(info, &mut text)) == sys::DUCKDB_V2_ERROR_NONE
            {
                let bytes = sys::str_bytes(text);
                if !bytes.is_empty() {
                    message = String::from_utf8_lossy(bytes).into_owned();
                }
            }
            ffi!(duckdb_v2_error_info_destroy(&mut info));
        }
    }
    Err(Error::new(code, message))
}

/// Runs a callback body, converting both errors and panics into a report on DuckDB's error slot.
/// A panic must not unwind into DuckDB, and would abort the process at the `extern "C"` boundary.
///
/// # Safety
///
/// `slot` must be the live error slot DuckDB passed to the running callback (or null).
pub(crate) unsafe fn guard(
    slot: *mut sys::duckdb_v2_error_info_handle,
    body: impl FnOnce() -> Result<()>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    let error = match outcome {
        Ok(Ok(())) => return,
        Ok(Err(e)) => e,
        Err(panic) => {
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            Error::internal(format!("plume panicked: {what}"))
        }
    };
    // SAFETY: forwarded from the caller.
    unsafe { error.report(slot) };
}
