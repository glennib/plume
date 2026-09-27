//! Process-wide defaults for `show()`.
//!
//! A v2 C-API extension cannot register `SET` options, so the defaults live here, for the whole process.
//! They start from the environment (`DUCKERS_VIEWER`, `DUCKERS_WAIT`) the first time they are read,
//! and [`set`] changes them, which is what `duckers_set(key, value)` calls.
//!
//! | Key | Environment | Values | Default |
//! |---|---|---|---|
//! | `viewer` | `DUCKERS_VIEWER` | `auto`, `terminal`, `window`, `browser` | `auto` |
//! | `wait` | `DUCKERS_WAIT` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `false` |
//! | `max_show` | | a non-negative integer; `0` shows nothing | `10` |
//!
//! An invalid environment value is not silently replaced by the default: [`current`] returns the error,
//! naming the variable, until [`set`] assigns that key a valid value.

use std::fmt;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::Viewer;
use crate::env::{self, Env};

/// The environment variable for the default viewer.
pub const VIEWER_ENV: &str = "DUCKERS_VIEWER";
/// The environment variable for the default `wait`.
pub const WAIT_ENV: &str = "DUCKERS_WAIT";

/// The keys [`set`] and [`get`] accept.
pub const KEYS: [&str; 3] = ["viewer", "wait", "max_show"];

const DEFAULT_MAX_SHOW: u64 = 10;

/// A snapshot of the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub viewer: Viewer,
    pub wait: bool,
    /// How many rows of one result `show()` displays at most.
    pub max_show: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            viewer: Viewer::Auto,
            wait: false,
            max_show: DEFAULT_MAX_SHOW,
        }
    }
}

/// A bad key or value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsError {
    /// `key` is not one of [`KEYS`].
    UnknownKey(String),
    /// `value` is not valid for `key`.
    InvalidValue {
        key: &'static str,
        value: String,
        expected: &'static str,
    },
    /// The environment variable `var` holds a value that is not valid.
    InvalidEnv {
        var: &'static str,
        value: String,
        expected: &'static str,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::UnknownKey(key) => write!(
                f,
                "unknown duckers setting '{key}'; the settings are 'viewer', 'wait' and 'max_show'"
            ),
            SettingsError::InvalidValue {
                key,
                value,
                expected,
            } => write!(
                f,
                "invalid value '{value}' for '{key}': expected {expected}"
            ),
            SettingsError::InvalidEnv {
                var,
                value,
                expected,
            } => write!(
                f,
                "environment variable {var}='{value}' is invalid: expected {expected}; \
                 fix or unset it, or override it with duckers_set"
            ),
        }
    }
}

impl std::error::Error for SettingsError {}

const WAIT_EXPECTED: &str = "true or false (also 1/0, yes/no, on/off)";
const MAX_SHOW_EXPECTED: &str = "a non-negative integer";

/// Parses a `wait` value.
pub fn parse_wait(value: &str) -> Result<bool, SettingsError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(SettingsError::InvalidValue {
            key: "wait",
            value: value.to_owned(),
            expected: WAIT_EXPECTED,
        }),
    }
}

/// Parses a `max_show` value.
pub fn parse_max_show(value: &str) -> Result<u64, SettingsError> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| SettingsError::InvalidValue {
            key: "max_show",
            value: value.to_owned(),
            expected: MAX_SHOW_EXPECTED,
        })
}

/// The defaults with each key's source of error kept apart, so that fixing one key
/// does not hide an invalid environment value for another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    viewer: Result<Viewer, SettingsError>,
    wait: Result<bool, SettingsError>,
    max_show: u64,
}

impl State {
    pub(crate) fn from_env(env: Env<'_>) -> State {
        let from_env = |var: &'static str, err: SettingsError| match err {
            SettingsError::InvalidValue {
                value, expected, ..
            } => SettingsError::InvalidEnv {
                var,
                value,
                expected,
            },
            other => other,
        };
        let viewer = match env::get(env, VIEWER_ENV) {
            None => Ok(Viewer::Auto),
            Some(v) => v.parse::<Viewer>().map_err(|e| from_env(VIEWER_ENV, e)),
        };
        let wait = match env::get(env, WAIT_ENV) {
            None => Ok(false),
            Some(v) => parse_wait(&v).map_err(|e| from_env(WAIT_ENV, e)),
        };
        State {
            viewer,
            wait,
            max_show: DEFAULT_MAX_SHOW,
        }
    }

    pub(crate) fn settings(&self) -> Result<Settings, SettingsError> {
        Ok(Settings {
            viewer: self.viewer.clone()?,
            wait: self.wait.clone()?,
            max_show: self.max_show,
        })
    }

    pub(crate) fn set(&mut self, key: &str, value: &str) -> Result<(), SettingsError> {
        match normalise_key(key)? {
            "viewer" => self.viewer = Ok(value.parse()?),
            "wait" => self.wait = Ok(parse_wait(value)?),
            "max_show" => self.max_show = parse_max_show(value)?,
            _ => unreachable!(),
        }
        Ok(())
    }

    pub(crate) fn get(&self, key: &str) -> Result<String, SettingsError> {
        Ok(match normalise_key(key)? {
            "viewer" => self.viewer.clone()?.name().to_owned(),
            "wait" => self.wait.clone()?.to_string(),
            "max_show" => self.max_show.to_string(),
            _ => unreachable!(),
        })
    }
}

fn normalise_key(key: &str) -> Result<&'static str, SettingsError> {
    let k = key.trim();
    KEYS.into_iter()
        .find(|known| known.eq_ignore_ascii_case(k))
        .ok_or_else(|| SettingsError::UnknownKey(key.to_owned()))
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(State::from_env(&env::process)))
        .lock()
        // The state is replaced whole by each operation, so a poisoned lock still holds a valid value.
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The current defaults.
pub fn current() -> Result<Settings, SettingsError> {
    state().settings()
}

/// Sets `key` to `value`, both as given to `duckers_set`. Keys and names are case-insensitive.
/// On error nothing changes.
pub fn set(key: &str, value: &str) -> Result<(), SettingsError> {
    state().set(key, value)
}

/// The current value of `key`, formatted as [`set`] accepts it.
pub fn get(key: &str) -> Result<String, SettingsError> {
    state().get(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::fixed;

    #[test]
    fn defaults_without_environment() {
        let s = State::from_env(&fixed(&[])).settings().unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(s.max_show, 10);
    }

    #[test]
    fn environment_sets_viewer_and_wait() {
        let env = fixed(&[("DUCKERS_VIEWER", "Browser"), ("DUCKERS_WAIT", "yes")]);
        let s = State::from_env(&env).settings().unwrap();
        assert_eq!(s.viewer, Viewer::Browser);
        assert!(s.wait);
    }

    #[test]
    fn empty_environment_value_means_unset() {
        let env = fixed(&[("DUCKERS_VIEWER", ""), ("DUCKERS_WAIT", "")]);
        assert_eq!(
            State::from_env(&env).settings().unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn invalid_environment_is_reported_until_overridden() {
        let env = fixed(&[("DUCKERS_VIEWER", "kitty")]);
        let mut st = State::from_env(&env);
        let e = st.settings().unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("DUCKERS_VIEWER='kitty'"), "{msg}");
        assert!(msg.contains("'terminal'"), "{msg}");
        assert!(st.get("viewer").is_err());
        // Other keys stay readable.
        assert_eq!(st.get("wait").unwrap(), "false");
        st.set("viewer", "terminal").unwrap();
        assert_eq!(st.settings().unwrap().viewer, Viewer::Terminal);
    }

    #[test]
    fn set_validates_and_keeps_old_value_on_error() {
        let mut st = State::from_env(&fixed(&[]));
        st.set("WAIT", " On ").unwrap();
        assert_eq!(st.get("wait").unwrap(), "true");
        let e = st.set("wait", "maybe").unwrap_err();
        assert_eq!(
            e.to_string(),
            "invalid value 'maybe' for 'wait': expected true or false (also 1/0, yes/no, on/off)"
        );
        assert_eq!(st.get("wait").unwrap(), "true");

        st.set("max_show", "0").unwrap();
        assert_eq!(st.settings().unwrap().max_show, 0);
        assert!(st.set("max_show", "-1").is_err());
        assert!(st.set("max_show", "ten").is_err());
        assert_eq!(st.get("max_show").unwrap(), "0");
    }

    #[test]
    fn unknown_key_lists_the_keys() {
        let mut st = State::from_env(&fixed(&[]));
        let e = st.set("colour", "red").unwrap_err().to_string();
        assert!(e.contains("'colour'") && e.contains("'max_show'"), "{e}");
    }

    #[test]
    fn wait_spellings() {
        for t in ["true", "TRUE", "1", "yes", "on"] {
            assert!(parse_wait(t).unwrap(), "{t}");
        }
        for f in ["false", "0", "No", "off"] {
            assert!(!parse_wait(f).unwrap(), "{f}");
        }
        assert!(parse_wait("").is_err());
    }
}
