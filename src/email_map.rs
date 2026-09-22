//! Recipient address mapping table, read once at startup from a TOML file.
//!
//! The mapping is applied as part of address normalization: a validated
//! address is normalized (trimmed and lowercased) and, if it matches a key
//! in the table, the email is sent to the mapped value instead.
//!
//! The file is optional: when it is missing or unparseable the service logs
//! a single warning and continues with an empty table (no mapping applied).

use std::collections::HashMap;

use tracing::warn;

use crate::validate::normalize;

/// Mapping from normalized recipient address to the address the email is
/// actually delivered to. Empty means "no mapping".
#[derive(Default)]
pub struct EmailMap {
    map: HashMap<String, String>,
}

impl EmailMap {
    /// Loads the table from `path`. Keys and values are normalized while
    /// loading, so matching is case-insensitive.
    ///
    /// Both unreadable files and parse errors are logged at `WARN` and
    /// treated as an empty table.
    pub fn load(path: &str) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                warn!(path, error = %err, "could not read email map, treating it as empty");
                return Self::default();
            }
        };
        match toml::from_str::<HashMap<String, String>>(&text) {
            Ok(raw) => Self {
                map: raw
                    .into_iter()
                    .map(|(key, value)| (normalize(&key), normalize(&value)))
                    .collect(),
            },
            Err(err) => {
                warn!(path, error = %err, "could not parse email map, treating it as empty");
                Self::default()
            }
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns the mapped target for `address`, if any. Lookup is
    /// case-insensitive.
    pub fn redirect_for(&self, address: &str) -> Option<&str> {
        self.map.get(&normalize(address)).map(String::as_str)
    }

    /// Applies normalization and mapping to `address`, returning the address
    /// the email should actually be sent to. The mapping counts as part of
    /// normalization, so rate limits key on the returned value.
    pub fn resolve(&self, address: &str) -> String {
        let normalized = normalize(address);
        self.map.get(&normalized).cloned().unwrap_or(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn missing_file_yields_empty_table() {
        let map = EmailMap::load("/nonexistent/path/to/email_map.toml");
        assert!(map.is_empty());
    }

    #[test]
    fn malformed_file_yields_empty_table() {
        let path = temp_path("hut_email_test_bad_map.toml");
        std::fs::write(&path, "not [valid toml\n").unwrap();
        assert!(EmailMap::load(path.to_str().unwrap()).is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn non_string_values_yield_empty_table() {
        let path = temp_path("hut_email_test_typed_map.toml");
        std::fs::write(&path, "\"a@x.com\" = 3\n").unwrap();
        assert!(EmailMap::load(path.to_str().unwrap()).is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn keys_and_values_are_normalized() {
        let path = temp_path("hut_email_test_norm_map.toml");
        std::fs::write(&path, "\"A@X.com\" = \"B@Y.com\"\n").unwrap();
        let map = EmailMap::load(path.to_str().unwrap());
        assert_eq!(map.resolve("a@x.com"), "b@y.com");
        assert_eq!(map.redirect_for("a@X.COM"), Some("b@y.com"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resolve_falls_back_to_normalized_address() {
        let map = EmailMap::default();
        assert_eq!(map.resolve("  A@X.com  "), "a@x.com");
        assert_eq!(map.redirect_for("a@x.com"), None);
    }
}
