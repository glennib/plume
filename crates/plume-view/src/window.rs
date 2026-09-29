//! A native window showing the image, built on `minifb`.
//!
//! A `minifb` window belongs to the thread that created it and pumps only its own events, so each window
//! gets a thread of its own and there is no process-wide event loop.
//!
//! - **Linux and the BSDs** use X11, which Wayland desktops provide through Xwayland. The `wayland` feature
//!   adds native Wayland windows, at the price of a libwayland warning on stderr each time one is closed
//!   (see `Cargo.toml`).
//! - **Linux and Windows:** the window thread is spawned for both modes. With `wait = false` the call returns
//!   once the window is up, and the thread keeps redrawing it until the user closes it or the process exits.
//!   With `wait = true` the call also joins the thread.
//! - **macOS:** AppKit requires the process main thread, and nothing pumps events once the call returns, so
//!   only `wait = true` on the main thread works; the window then runs on the calling thread.
//!   Every other case is reported unavailable, and `auto` moves on to the browser.
//!
//! Process exit with windows open: window threads are detached and never joined, and the `minifb` window is
//! a local of its thread, so exit does not run its destructor; the operating system removes the window with the
//! process. On Unix an `atexit` handler additionally stops window threads between frames, so none is inside
//! Xlib or libwayland while the rest of the process tears down.

use crate::{Image, ShowError, Viewer};

/// Whether a window can be shown here with this `wait`, judged from the platform, the environment and the
/// calling thread. `Err` carries the reason.
pub fn probe(wait: bool) -> Result<(), String> {
    availability(
        Platform::current(),
        cfg!(feature = "window"),
        &crate::env::process,
        wait,
        is_main_thread(),
    )
}

/// Shows `image` in a window. With `wait`, returns once the user has closed it.
pub fn show(image: &Image, wait: bool) -> Result<(), ShowError> {
    probe(wait).map_err(|reason| ShowError::unavailable(Viewer::Window, reason))?;
    let rgb = image.rgb()?;
    let pixels: Vec<u32> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
        .collect();
    imp::open(
        pixels,
        image.width() as usize,
        image.height() as usize,
        wait,
    )
    .map_err(|message| ShowError::failed(Viewer::Window, message))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    MacOs,
    Windows,
    /// Linux and the BSDs: X11 or Wayland.
    Unix,
    Other,
}

impl Platform {
    fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else if cfg!(unix) {
            Platform::Unix
        } else {
            Platform::Other
        }
    }
}

fn availability(
    platform: Platform,
    built_with_window: bool,
    env: crate::env::Env<'_>,
    wait: bool,
    main_thread: bool,
) -> Result<(), String> {
    use crate::env::has;
    if !built_with_window {
        return Err("plume was built without the window viewer".into());
    }
    match platform {
        Platform::Windows => Ok(()),
        Platform::Unix if cfg!(feature = "wayland") => {
            if has(env, "WAYLAND_DISPLAY") || has(env, "DISPLAY") {
                Ok(())
            } else {
                Err("no display: neither WAYLAND_DISPLAY nor DISPLAY is set".into())
            }
        }
        Platform::Unix => {
            if has(env, "DISPLAY") {
                Ok(())
            } else if has(env, "WAYLAND_DISPLAY") {
                Err("no X11 display: DISPLAY is not set, and windows need X11 \
                     (on Wayland, through Xwayland)"
                    .into())
            } else {
                Err("no display: neither WAYLAND_DISPLAY nor DISPLAY is set".into())
            }
        }
        Platform::MacOs if !wait => Err(
            "on macOS a window that does not block needs an event loop on the main thread, \
             which the host does not run; use wait := true"
                .into(),
        ),
        Platform::MacOs if !main_thread => Err(
            "on macOS a window must be opened on the process main thread, and this call runs on \
             another; SET threads = 1 usually keeps a single-row query on the main thread"
                .into(),
        ),
        Platform::MacOs => Ok(()),
        Platform::Other => Err("no window support on this platform".into()),
    }
}

#[cfg(target_os = "macos")]
fn is_main_thread() -> bool {
    // SAFETY: pthread_main_np has no preconditions.
    unsafe { libc::pthread_main_np() == 1 }
}

/// Only macOS restricts windows to the main thread, so elsewhere the answer is not needed.
#[cfg(not(target_os = "macos"))]
fn is_main_thread() -> bool {
    false
}

#[cfg(feature = "window")]
mod imp {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use minifb::{Key, ScaleMode, Window, WindowOptions};

    use super::exit_guard;

    /// The pause between redraws. X11 has no stored surface content, so the window is redrawn
    /// periodically rather than on expose events; the same loop pumps input events.
    const FRAME: Duration = Duration::from_millis(33);

    pub fn open(pixels: Vec<u32>, width: usize, height: usize, wait: bool) -> Result<(), String> {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        let title = format!("plume chart {n}");

        if cfg!(target_os = "macos") {
            // `probe` has established that this is the main thread and that `wait` is set.
            return run(&title, &pixels, width, height, |_| {});
        }

        exit_guard::install();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name(format!("plume-window-{n}"))
            .spawn(move || {
                let _ = run(&title, &pixels, width, height, |r| {
                    let _ = ready_tx.send(r);
                });
            })
            .map_err(|e| format!("cannot start the window thread: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err("the window thread ended before opening a window".into()),
        }
        if wait {
            handle
                .join()
                .map_err(|_| "the window thread panicked".to_string())?;
        }
        Ok(())
    }

    /// Opens the window, reports the outcome through `ready`, then redraws until the window is closed
    /// (title-bar button or Escape).
    fn run(
        title: &str,
        pixels: &[u32],
        width: usize,
        height: usize,
        ready: impl FnOnce(Result<(), String>),
    ) -> Result<(), String> {
        let options = WindowOptions {
            resize: true,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        };
        let opened = exit_guard::frame(|| -> Result<Window, String> {
            let mut window = Window::new(title, width, height, options)
                .map_err(|e| format!("cannot open a window: {e}"))?;
            window.set_target_fps(0);
            window
                .update_with_buffer(pixels, width, height)
                .map_err(|e| format!("cannot draw into the window: {e}"))?;
            Ok(window)
        });
        let mut window = match opened {
            Ok(w) => {
                ready(Ok(()));
                w
            }
            Err(e) => {
                ready(Err(e.clone()));
                return Err(e);
            }
        };
        loop {
            std::thread::sleep(FRAME);
            let open = exit_guard::frame(|| {
                window.is_open()
                    && !window.is_key_down(Key::Escape)
                    && window.update_with_buffer(pixels, width, height).is_ok()
            });
            if !open {
                return Ok(());
            }
        }
    }
}

#[cfg(not(feature = "window"))]
mod imp {
    pub fn open(_: Vec<u32>, _: usize, _: usize, _: bool) -> Result<(), String> {
        Err("plume was built without the window viewer".into())
    }
}

/// Keeps window threads out of the windowing libraries once the process has started to exit.
///
/// Each call into `minifb` runs inside [`frame`], which counts it as busy. The `atexit` handler marks the
/// process as exiting and waits, at most [`EXIT_GRACE`], for busy calls to finish; a window thread that
/// finds the mark set parks forever instead of starting another call. The windows are never dropped:
/// the operating system closes them with the process.
///
/// On Windows, `ExitProcess` terminates other threads before any `atexit` handler of a DLL runs, so no
/// handler is needed there.
#[cfg(feature = "window")]
mod exit_guard {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

    static EXITING: AtomicBool = AtomicBool::new(false);
    static BUSY: AtomicUsize = AtomicUsize::new(0);

    /// How long the exit handler waits for a window thread to finish its current call.
    #[cfg_attr(not(unix), allow(dead_code))]
    const EXIT_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

    /// Registers the exit handler once per process.
    pub fn install() {
        #[cfg(unix)]
        {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                // SAFETY: `on_exit` is an `extern "C" fn()` that never unwinds.
                unsafe {
                    libc::atexit(on_exit);
                }
            });
        }
    }

    #[cfg(unix)]
    extern "C" fn on_exit() {
        EXITING.store(true, SeqCst);
        let deadline = std::time::Instant::now() + EXIT_GRACE;
        while BUSY.load(SeqCst) > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Runs `f` unless the process is exiting, in which case the calling thread parks for good.
    pub fn frame<T>(f: impl FnOnce() -> T) -> T {
        struct Busy;
        impl Drop for Busy {
            fn drop(&mut self) {
                BUSY.fetch_sub(1, SeqCst);
            }
        }
        BUSY.fetch_add(1, SeqCst);
        let busy = Busy;
        if EXITING.load(SeqCst) {
            drop(busy);
            loop {
                std::thread::park();
            }
        }
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::fixed;

    fn check(
        platform: Platform,
        pairs: &[(&str, &str)],
        wait: bool,
        main: bool,
    ) -> Result<(), String> {
        availability(platform, true, &fixed(pairs), wait, main)
    }

    #[test]
    fn unix_needs_a_display() {
        let wayland_only = check(
            Platform::Unix,
            &[("WAYLAND_DISPLAY", "wayland-1")],
            false,
            false,
        );
        if cfg!(feature = "wayland") {
            assert!(wayland_only.is_ok());
        } else {
            let e = wayland_only.unwrap_err();
            assert!(e.contains("Xwayland"), "{e}");
        }
        assert!(check(Platform::Unix, &[("DISPLAY", ":0")], true, false).is_ok());
        let e = check(Platform::Unix, &[("DISPLAY", "")], false, false).unwrap_err();
        assert!(e.contains("no display"), "{e}");
    }

    #[test]
    fn windows_always_works() {
        assert!(check(Platform::Windows, &[], false, false).is_ok());
        assert!(check(Platform::Windows, &[], true, false).is_ok());
    }

    #[test]
    fn macos_needs_wait_and_the_main_thread() {
        let e = check(Platform::MacOs, &[], false, true).unwrap_err();
        assert!(e.contains("wait := true"), "{e}");
        let e = check(Platform::MacOs, &[], true, false).unwrap_err();
        assert!(e.contains("main thread"), "{e}");
        assert!(check(Platform::MacOs, &[], true, true).is_ok());
    }

    #[test]
    fn without_the_feature_nothing_works() {
        let e = availability(Platform::Windows, false, &fixed(&[]), true, true).unwrap_err();
        assert!(e.contains("without the window viewer"), "{e}");
    }
}
