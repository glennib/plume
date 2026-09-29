//! The image as an HTML page in the default browser.
//!
//! The page embeds the PNG as a `data:` URI, so it is a single self-contained file. It is written to the
//! per-user cache directory ([`cache_dir`]) and handed to the platform opener: `xdg-open` on Linux and the
//! BSDs, `open` on macOS, `ShellExecuteW` on Windows. The opener is not waited for, since `xdg-open` can
//! stay in the foreground for as long as the browser runs.
//!
//! Pages older than a day are removed from the cache directory whenever a new page is written.
//! Snap-packaged browsers cannot read `~/.cache`; with those the tab shows a file-access error.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::env::{self, Env};
use crate::{Image, ShowError, Viewer};

/// How long a written page is kept.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether a browser can be opened here. `Err` carries the reason.
///
/// On Linux and the BSDs a graphical session is required (`WAYLAND_DISPLAY` or `DISPLAY`): without one,
/// `xdg-open` falls back to a text-mode browser that would take over the terminal the shell is using.
pub fn probe() -> Result<(), String> {
    availability(&env::process)
}

fn availability(env: Env<'_>) -> Result<(), String> {
    if cfg!(any(target_os = "macos", windows)) {
        return Ok(());
    }
    if env::has(env, "WAYLAND_DISPLAY") || env::has(env, "DISPLAY") {
        Ok(())
    } else {
        Err("no graphical session: neither WAYLAND_DISPLAY nor DISPLAY is set".into())
    }
}

/// The directory pages are written to: `plume` inside `$XDG_CACHE_HOME` (else `~/.cache`) on Linux and
/// the BSDs, `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on Windows, and the temporary directory when
/// none of those is known.
pub fn cache_dir() -> PathBuf {
    cache_base(&env::process)
        .unwrap_or_else(std::env::temp_dir)
        .join("plume")
}

fn cache_base(env: Env<'_>) -> Option<PathBuf> {
    let abs = |v: String| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
    if cfg!(windows) {
        env::get(env, "LOCALAPPDATA").and_then(abs)
    } else if cfg!(target_os = "macos") {
        env::get(env, "HOME")
            .and_then(abs)
            .map(|h| h.join("Library").join("Caches"))
    } else {
        env::get(env, "XDG_CACHE_HOME").and_then(abs).or_else(|| {
            env::get(env, "HOME")
                .and_then(abs)
                .map(|h| h.join(".cache"))
        })
    }
}

/// Writes the page for `image` and opens it in the browser. Returns the page's path.
pub fn show(image: &Image) -> Result<PathBuf, ShowError> {
    probe().map_err(|reason| ShowError::unavailable(Viewer::Browser, reason))?;
    let html = page(image)?;
    let dir = cache_dir();
    let failed = |what: &str, p: &Path, e: std::io::Error| {
        ShowError::failed(Viewer::Browser, format!("{what} {}: {e}", p.display()))
    };
    std::fs::create_dir_all(&dir).map_err(|e| failed("cannot create", &dir, e))?;
    remove_old_pages(&dir);
    let path = dir.join(file_name());
    std::fs::write(&path, html).map_err(|e| failed("cannot write", &path, e))?;
    open(&path).map_err(|message| ShowError::failed(Viewer::Browser, message))?;
    Ok(path)
}

/// The HTML page showing `image`, scaled down to fit the browser window if it is larger.
pub fn page(image: &Image) -> Result<String, crate::ImageError> {
    let data = BASE64.encode(image.png()?);
    Ok(format!(
        "<!DOCTYPE html>\n\
         <html><head><meta charset=\"utf-8\"><title>plume chart</title>\n\
         <style>html,body{{margin:0;background:#fff}}\
         body{{display:flex;min-height:100vh;align-items:center;justify-content:center}}\
         img{{max-width:100vw;max-height:100vh;width:auto;height:auto}}</style></head>\n\
         <body><img alt=\"plume chart\" width=\"{w}\" height=\"{h}\" src=\"data:image/png;base64,{data}\"></body></html>\n",
        w = image.width(),
        h = image.height(),
    ))
}

fn file_name() -> String {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("chart-{millis}-{}-{n}.html", std::process::id())
}

/// Removes `chart-*.html` pages older than [`KEEP`]. Errors are ignored: this is housekeeping.
fn remove_old_pages(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("chart-") && name.ends_with(".html")) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open(path: &Path) -> Result<(), String> {
    spawn_detached("xdg-open", path)
}

#[cfg(target_os = "macos")]
fn open(path: &Path) -> Result<(), String> {
    spawn_detached("open", path)
}

/// Starts `program path` without waiting for it; a thread reaps it when it exits.
#[cfg(unix)]
fn spawn_detached(program: &str, path: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    let _ = std::thread::Builder::new()
        .name("plume-browser-open".into())
        .spawn(move || child.wait());
    Ok(())
}

#[cfg(windows)]
fn open(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain([0]).collect::<Vec<u16>>();
    let verb = wide("open".as_ref());
    let file = wide(path.as_os_str());
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW reports success with a value greater than 32.
    if result as usize > 32 {
        Ok(())
    } else {
        Err(format!(
            "ShellExecuteW could not open {} (code {})",
            path.display(),
            result as usize
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn open(_: &Path) -> Result<(), String> {
    Err("no way to open a browser on this platform".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(unix, not(target_os = "macos")))]
    use crate::env::fixed;

    #[test]
    fn page_embeds_the_png() {
        let img = Image::from_rgb(2, 1, vec![0, 0, 0, 255, 255, 255]).unwrap();
        let html = page(&img).unwrap();
        let b64 = BASE64.encode(img.png().unwrap());
        assert!(html.contains(&format!("src=\"data:image/png;base64,{b64}\"")));
        assert!(html.contains("width=\"2\" height=\"1\""));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_cache_dir_follows_xdg() {
        let env = fixed(&[("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/u")]);
        assert_eq!(cache_base(&env), Some(PathBuf::from("/xdg")));
        let env = fixed(&[("XDG_CACHE_HOME", "relative"), ("HOME", "/home/u")]);
        assert_eq!(cache_base(&env), Some(PathBuf::from("/home/u/.cache")));
        assert_eq!(cache_base(&fixed(&[])), None);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_needs_a_graphical_session() {
        assert!(availability(&fixed(&[("DISPLAY", ":1")])).is_ok());
        let e = availability(&fixed(&[("TERM", "xterm")])).unwrap_err();
        assert!(e.contains("no graphical session"), "{e}");
    }

    #[test]
    fn file_names_are_unique() {
        assert_ne!(file_name(), file_name());
    }
}
