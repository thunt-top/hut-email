//! Service configuration, loaded from TOML files in the config directory.
//!
//! `config.toml` holds versioned, non-secret settings (committed to the
//! repository); `secret.toml` holds the Tencent Cloud credentials and is
//! gitignored. Both files are required — a missing or malformed file fails
//! startup with an actionable error. The email map (`email_map.toml`) is
//! handled separately by [`crate::email_map`] and remains optional.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(default)]
struct ConfigFile {
    from_address: String,
    listen_addr: String,
    ses: Ses,
    rate_limit: RateLimit,
}

#[derive(Deserialize)]
#[serde(default)]
struct Ses {
    endpoint: String,
    region: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct RateLimit {
    max_per_hour: u32,
}

#[derive(Deserialize)]
struct SecretFile {
    secret_id: String,
    secret_key: String,
}

/// Fully resolved service configuration.
pub struct Config {
    pub secret_id: String,
    pub secret_key: String,
    pub from_address: String,
    pub endpoint: String,
    pub region: String,
    pub listen_addr: String,
    pub rate_limit_max_per_hour: u32,
    pub email_map_path: PathBuf,
}

// Secrets must never show up in logs or debug output.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("secret_id", &"<redacted>")
            .field("secret_key", &"<redacted>")
            .field("from_address", &self.from_address)
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("listen_addr", &self.listen_addr)
            .field("rate_limit_max_per_hour", &self.rate_limit_max_per_hour)
            .field("email_map_path", &self.email_map_path)
            .finish()
    }
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            from_address: String::new(),
            listen_addr: "0.0.0.0:39788".to_string(),
            ses: Ses::default(),
            rate_limit: RateLimit::default(),
        }
    }
}

impl Default for Ses {
    fn default() -> Self {
        Self {
            endpoint: "ses.tencentcloudapi.com".to_string(),
            region: "ap-hongkong".to_string(),
        }
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self { max_per_hour: 20 }
    }
}

impl Config {
    /// Loads and validates `config.toml` and `secret.toml` from `config_dir`.
    pub fn load(config_dir: &str) -> Result<Self, String> {
        let dir = Path::new(config_dir);
        let config_path = dir.join("config.toml");
        let secret_path = dir.join("secret.toml");

        let config_text = read_required(&config_path)?;
        let config_file: ConfigFile = toml::from_str(&config_text)
            .map_err(|err| format!("could not parse {}: {err}", config_path.display()))?;
        if config_file.from_address.trim().is_empty() {
            return Err(format!(
                "`from_address` is required in {}",
                config_path.display()
            ));
        }
        if config_file.rate_limit.max_per_hour == 0 {
            return Err(format!(
                "`rate_limit.max_per_hour` must be a positive integer in {}",
                config_path.display()
            ));
        }

        let secret_text = read_required(&secret_path).map_err(|err| {
            format!(
                "{err}; copy config/secret.example.toml to {} and fill in your Tencent Cloud credentials",
                secret_path.display()
            )
        })?;
        let secret_file: SecretFile = toml::from_str(&secret_text)
            .map_err(|err| format!("could not parse {}: {err}", secret_path.display()))?;
        if secret_file.secret_id.trim().is_empty() || secret_file.secret_key.trim().is_empty() {
            return Err(format!(
                "`secret_id` and `secret_key` must be non-empty in {}",
                secret_path.display()
            ));
        }

        Ok(Self {
            secret_id: secret_file.secret_id,
            secret_key: secret_file.secret_key,
            from_address: config_file.from_address,
            endpoint: config_file.ses.endpoint,
            region: config_file.ses.region,
            listen_addr: config_file.listen_addr,
            rate_limit_max_per_hour: config_file.rate_limit.max_per_hour,
            email_map_path: dir.join("email_map.toml"),
        })
    }
}

fn read_required(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("could not read {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn remove_dir(dir: &Path) {
        std::fs::remove_dir_all(dir).ok();
    }

    const SECRET: &str = "secret_id = \"AKID\"\nsecret_key = \"SECRET\"\n";
    const MINIMAL_CONFIG: &str = "from_address = \"staff@no-reply.thunt.top\"\n";

    #[test]
    fn loads_full_config() {
        let dir = temp_dir("hut_email_cfg_full");
        write(
            &dir,
            "config.toml",
            "from_address = \"staff@no-reply.thunt.top\"\n\
             listen_addr = \"127.0.0.1:9999\"\n\n\
             [ses]\nendpoint = \"e\"\nregion = \"r\"\n\n\
             [rate_limit]\nmax_per_hour = 7\n",
        );
        write(&dir, "secret.toml", SECRET);
        let cfg = Config::load(dir.to_str().unwrap()).unwrap();
        assert_eq!(cfg.listen_addr, "127.0.0.1:9999");
        assert_eq!(cfg.endpoint, "e");
        assert_eq!(cfg.region, "r");
        assert_eq!(cfg.rate_limit_max_per_hour, 7);
        assert_eq!(cfg.email_map_path, dir.join("email_map.toml"));
        remove_dir(&dir);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let dir = temp_dir("hut_email_cfg_defaults");
        write(&dir, "config.toml", MINIMAL_CONFIG);
        write(&dir, "secret.toml", SECRET);
        let cfg = Config::load(dir.to_str().unwrap()).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:39788");
        assert_eq!(cfg.endpoint, "ses.tencentcloudapi.com");
        assert_eq!(cfg.region, "ap-hongkong");
        assert_eq!(cfg.rate_limit_max_per_hour, 20);
        remove_dir(&dir);
    }

    #[test]
    fn missing_config_file_is_error() {
        let dir = temp_dir("hut_email_cfg_missing_config");
        write(&dir, "secret.toml", SECRET);
        assert!(Config::load(dir.to_str().unwrap()).is_err());
        remove_dir(&dir);
    }

    #[test]
    fn missing_secret_file_points_at_the_example() {
        let dir = temp_dir("hut_email_cfg_missing_secret");
        write(&dir, "config.toml", MINIMAL_CONFIG);
        let err = Config::load(dir.to_str().unwrap()).unwrap_err();
        assert!(err.contains("secret.example.toml"), "{err}");
        remove_dir(&dir);
    }

    #[test]
    fn missing_from_address_is_error() {
        let dir = temp_dir("hut_email_cfg_no_from");
        write(&dir, "config.toml", "listen_addr = \"127.0.0.1:1\"\n");
        write(&dir, "secret.toml", SECRET);
        assert!(Config::load(dir.to_str().unwrap()).is_err());
        remove_dir(&dir);
    }

    #[test]
    fn zero_rate_limit_is_error() {
        let dir = temp_dir("hut_email_cfg_zero_rate");
        write(
            &dir,
            "config.toml",
            "from_address = \"a@b.c\"\n[rate_limit]\nmax_per_hour = 0\n",
        );
        write(&dir, "secret.toml", SECRET);
        assert!(Config::load(dir.to_str().unwrap()).is_err());
        remove_dir(&dir);
    }

    #[test]
    fn malformed_toml_is_error() {
        let dir = temp_dir("hut_email_cfg_bad_toml");
        write(&dir, "config.toml", "not [valid toml\n");
        write(&dir, "secret.toml", SECRET);
        assert!(Config::load(dir.to_str().unwrap()).is_err());
        remove_dir(&dir);
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let dir = temp_dir("hut_email_cfg_redact");
        write(&dir, "config.toml", MINIMAL_CONFIG);
        write(&dir, "secret.toml", SECRET);
        let cfg = Config::load(dir.to_str().unwrap()).unwrap();
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("AKID") && !dbg.contains("SECRET"), "{dbg}");
        assert!(dbg.contains("<redacted>"));
        remove_dir(&dir);
    }
}
