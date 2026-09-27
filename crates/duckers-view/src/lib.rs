//! Shows a rendered chart image to the user.
//!
//! Three viewers exist, tried in this order by [`Viewer::Auto`]:
//!
//! - [`Viewer::Terminal`]: an inline image (kitty graphics, iTerm2 or sixel) written to the controlling
//!   terminal, never to stdout. See [`terminal`].
//! - [`Viewer::Window`]: a native `minifb` window on a thread of its own. See [`window`].
//! - [`Viewer::Browser`]: an HTML page in the cache directory, opened in the default browser. See [`browser`].
//!
//! Whether a viewer can work is decided from the environment, the platform and the calling thread only.
//! Nothing is ever read from the terminal, because in a SQL shell the line editor owns stdin.
//!
//! Process-wide defaults for the viewer, `wait` and the multi-row cap live in [`settings`].

pub mod browser;
mod env;
mod image;
pub mod settings;
pub mod terminal;
pub mod window;

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

pub use crate::image::{Image, ImageError};
pub use crate::settings::{Settings, SettingsError};
pub use crate::terminal::Protocol;

/// Which viewer shows an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Viewer {
    /// The first of terminal, window and browser that can work here.
    Auto,
    Terminal,
    Window,
    Browser,
}

impl Viewer {
    /// Every viewer, in the order the names are listed in messages.
    pub const ALL: [Viewer; 4] = [
        Viewer::Auto,
        Viewer::Terminal,
        Viewer::Window,
        Viewer::Browser,
    ];

    /// The order in which [`Viewer::Auto`] tries the concrete viewers.
    pub const AUTO_ORDER: [Viewer; 3] = [Viewer::Terminal, Viewer::Window, Viewer::Browser];

    /// The lower-case name used in settings and SQL (`'terminal'`).
    pub fn name(self) -> &'static str {
        match self {
            Viewer::Auto => "auto",
            Viewer::Terminal => "terminal",
            Viewer::Window => "window",
            Viewer::Browser => "browser",
        }
    }
}

impl fmt::Display for Viewer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Viewer {
    type Err = SettingsError;

    /// Parses a viewer name, ignoring case and surrounding whitespace.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.trim();
        Viewer::ALL
            .into_iter()
            .find(|v| v.name().eq_ignore_ascii_case(wanted))
            .ok_or_else(|| SettingsError::InvalidValue {
                key: "viewer",
                value: s.to_owned(),
                expected: "one of 'auto', 'terminal', 'window', 'browser'",
            })
    }
}

/// How [`show`] displays an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShowOptions {
    pub viewer: Viewer,
    /// Block until the viewer is closed. Only the window viewer can block; the terminal and browser
    /// viewers return at once either way.
    pub wait: bool,
}

impl ShowOptions {
    /// The process-wide defaults from [`settings`].
    ///
    /// Fails when `DUCKERS_VIEWER` or `DUCKERS_WAIT` holds an invalid value that no
    /// [`settings::set`] call has replaced since.
    pub fn from_settings() -> Result<Self, SettingsError> {
        let s = settings::current()?;
        Ok(ShowOptions {
            viewer: s.viewer,
            wait: s.wait,
        })
    }
}

impl Default for ShowOptions {
    /// `auto` and no waiting, regardless of the process-wide settings.
    fn default() -> Self {
        ShowOptions {
            viewer: Viewer::Auto,
            wait: false,
        }
    }
}

/// What [`show`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// The image was written to the controlling terminal.
    Terminal(Protocol),
    /// A window was opened. With `waited`, it has also been closed again.
    Window { waited: bool },
    /// The page was written to `path` and the browser was asked to open it.
    Browser { path: PathBuf },
}

impl Shown {
    pub fn viewer(&self) -> Viewer {
        match self {
            Shown::Terminal(_) => Viewer::Terminal,
            Shown::Window { .. } => Viewer::Window,
            Shown::Browser { .. } => Viewer::Browser,
        }
    }
}

/// Why [`show`] could not show an image.
#[derive(Debug)]
pub enum ShowError {
    /// The viewer cannot work in this process, and nothing was attempted.
    Unavailable { viewer: Viewer, reason: String },
    /// The viewer was attempted and failed.
    Failed { viewer: Viewer, message: String },
    /// [`Viewer::Auto`] found no working viewer; one entry per viewer tried, in order.
    NoViewer(Vec<ShowError>),
    /// The image could not be encoded or decoded for the viewer.
    Image(ImageError),
}

impl ShowError {
    pub(crate) fn unavailable(viewer: Viewer, reason: impl Into<String>) -> Self {
        ShowError::Unavailable {
            viewer,
            reason: reason.into(),
        }
    }

    pub(crate) fn failed(viewer: Viewer, message: impl Into<String>) -> Self {
        ShowError::Failed {
            viewer,
            message: message.into(),
        }
    }
}

impl fmt::Display for ShowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShowError::Unavailable { viewer, reason } => {
                write!(f, "viewer '{viewer}' is unavailable: {reason}")
            }
            ShowError::Failed { viewer, message } => {
                write!(f, "viewer '{viewer}' failed: {message}")
            }
            ShowError::NoViewer(attempts) => {
                f.write_str("no viewer can show the chart here")?;
                for (i, a) in attempts.iter().enumerate() {
                    f.write_str(if i == 0 { ": " } else { "; " })?;
                    match a {
                        ShowError::Unavailable { viewer, reason } => {
                            write!(f, "{viewer}: {reason}")?
                        }
                        ShowError::Failed { viewer, message } => {
                            write!(f, "{viewer}: failed: {message}")?
                        }
                        other => write!(f, "{other}")?,
                    }
                }
                Ok(())
            }
            ShowError::Image(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ShowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ShowError::Image(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ImageError> for ShowError {
    fn from(e: ImageError) -> Self {
        ShowError::Image(e)
    }
}

/// Shows `image` with the viewer in `options`.
///
/// With [`Viewer::Auto`], every viewer in [`Viewer::AUTO_ORDER`] is tried until one succeeds; a viewer that is
/// unavailable or fails is skipped, and if none works the error lists the reason for each.
/// A concrete viewer is tried alone and its own error is returned.
pub fn show(image: &Image, options: &ShowOptions) -> Result<Shown, ShowError> {
    match options.viewer {
        Viewer::Auto => {
            let mut attempts = Vec::new();
            for viewer in Viewer::AUTO_ORDER {
                match show_one(viewer, image, options.wait) {
                    Ok(shown) => return Ok(shown),
                    Err(e @ ShowError::Image(_)) => return Err(e),
                    Err(e) => attempts.push(e),
                }
            }
            Err(ShowError::NoViewer(attempts))
        }
        viewer => show_one(viewer, image, options.wait),
    }
}

fn show_one(viewer: Viewer, image: &Image, wait: bool) -> Result<Shown, ShowError> {
    match viewer {
        Viewer::Terminal => terminal::show(image).map(Shown::Terminal),
        Viewer::Window => window::show(image, wait).map(|()| Shown::Window { waited: wait }),
        Viewer::Browser => browser::show(image).map(|path| Shown::Browser { path }),
        Viewer::Auto => unreachable!("auto is resolved by show()"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_names_round_trip() {
        for v in Viewer::ALL {
            assert_eq!(v.name().parse::<Viewer>().unwrap(), v);
        }
        assert_eq!(" Window ".parse::<Viewer>().unwrap(), Viewer::Window);
    }

    #[test]
    fn unknown_viewer_lists_the_choices() {
        let e = "sixel".parse::<Viewer>().unwrap_err().to_string();
        assert!(e.contains("'sixel'"), "{e}");
        assert!(e.contains("'auto', 'terminal', 'window', 'browser'"), "{e}");
    }

    #[test]
    fn no_viewer_message_names_each_reason() {
        let e = ShowError::NoViewer(vec![
            ShowError::unavailable(Viewer::Terminal, "running inside tmux"),
            ShowError::failed(Viewer::Window, "no display"),
            ShowError::unavailable(Viewer::Browser, "no display"),
        ]);
        assert_eq!(
            e.to_string(),
            "no viewer can show the chart here: terminal: running inside tmux; \
             window: failed: no display; browser: no display"
        );
    }
}
