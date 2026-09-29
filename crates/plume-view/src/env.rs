//! Environment lookup that tests can replace.
//!
//! Every probe takes an `&dyn Fn(&str) -> Option<String>` so tests inject a fixed environment
//! instead of mutating the process one.

/// A lookup from variable name to value.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process environment. Non-UTF-8 values are converted lossily.
pub fn process(key: &str) -> Option<String> {
    std::env::var_os(key).map(|v| v.to_string_lossy().into_owned())
}

/// Looks up `key`, treating an empty value as unset.
pub fn get(env: Env<'_>, key: &str) -> Option<String> {
    env(key).filter(|v| !v.is_empty())
}

/// Whether `key` is set to a non-empty value.
pub fn has(env: Env<'_>, key: &str) -> bool {
    get(env, key).is_some()
}

/// Builds an environment from fixed pairs, for tests.
#[cfg(test)]
pub fn fixed(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}
