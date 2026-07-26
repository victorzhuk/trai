use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug)]
pub struct Config {
    pub store_root: PathBuf,
    pub mic_source: String,
    pub monitor_source: String,
    pub mic_language: Option<String>,
    pub monitor_language: Option<String>,
    pub whisper_url: String,
    pub vad_threshold: f32,
    pub silence_hold_ms: u64,
    pub duration_cap_ms: u64,
    pub confidence_floor: f32,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Missing(&'static str),
    Invalid { key: &'static str, reason: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "failed to read config file: {e}"),
            ConfigError::Parse(e) => write!(f, "failed to parse config: {e}"),
            ConfigError::Missing(key) => write!(f, "missing required config key: `{key}`"),
            ConfigError::Invalid { key, reason } => {
                write!(f, "invalid value for `{key}`: {reason}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::Parse(e)
    }
}

fn invalid(key: &'static str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key,
        reason: reason.into(),
    }
}

// All fields are Option so a missing TOML key surfaces as a named
// `ConfigError::Missing` below rather than an opaque serde error.
#[derive(Deserialize)]
struct RawConfig {
    store_root: Option<PathBuf>,
    mic_source: Option<String>,
    monitor_source: Option<String>,
    mic_language: Option<String>,
    monitor_language: Option<String>,
    whisper_url: Option<String>,
    vad_threshold: Option<f32>,
    silence_hold_ms: Option<u64>,
    duration_cap_ms: Option<u64>,
    confidence_floor: Option<f32>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_toml_str(&contents)
    }

    pub fn from_toml_str(s: &str) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(s)?;

        let store_root = raw.store_root.ok_or(ConfigError::Missing("store_root"))?;
        let mic_source = raw.mic_source.ok_or(ConfigError::Missing("mic_source"))?;
        let monitor_source = raw
            .monitor_source
            .ok_or(ConfigError::Missing("monitor_source"))?;
        let whisper_url = raw.whisper_url.ok_or(ConfigError::Missing("whisper_url"))?;
        let vad_threshold = raw
            .vad_threshold
            .ok_or(ConfigError::Missing("vad_threshold"))?;
        let silence_hold_ms = raw
            .silence_hold_ms
            .ok_or(ConfigError::Missing("silence_hold_ms"))?;
        let duration_cap_ms = raw
            .duration_cap_ms
            .ok_or(ConfigError::Missing("duration_cap_ms"))?;
        let confidence_floor = raw
            .confidence_floor
            .ok_or(ConfigError::Missing("confidence_floor"))?;

        reqwest::Url::parse(&whisper_url)
            .map_err(|e| invalid("whisper_url", format!("not a valid URL: {e}")))?;

        if !(0.0..=1.0).contains(&confidence_floor) {
            return Err(invalid("confidence_floor", "must be within [0.0, 1.0]"));
        }
        if !(0.0..=1.0).contains(&vad_threshold) {
            return Err(invalid("vad_threshold", "must be within [0.0, 1.0]"));
        }
        if silence_hold_ms == 0 {
            return Err(invalid("silence_hold_ms", "must be greater than 0"));
        }
        if duration_cap_ms == 0 {
            return Err(invalid("duration_cap_ms", "must be greater than 0"));
        }

        Ok(Config {
            store_root,
            mic_source,
            monitor_source,
            mic_language: raw.mic_language,
            monitor_language: raw.monitor_language,
            whisper_url,
            vad_threshold,
            silence_hold_ms,
            duration_cap_ms,
            confidence_floor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_toml() -> String {
        r#"
            store_root = "/tmp/trai-store"
            mic_source = "alsa_input.default"
            monitor_source = "alsa_output.default.monitor"
            mic_language = "en"
            monitor_language = "de"
            whisper_url = "http://localhost:8080"
            vad_threshold = 0.5
            silence_hold_ms = 500
            duration_cap_ms = 30000
            confidence_floor = 0.6
        "#
        .to_string()
    }

    #[test]
    fn every_field_round_trips_from_toml() {
        let config = Config::from_toml_str(&valid_toml()).unwrap();

        assert_eq!(config.store_root, PathBuf::from("/tmp/trai-store"));
        assert_eq!(config.mic_source, "alsa_input.default");
        assert_eq!(config.monitor_source, "alsa_output.default.monitor");
        assert_eq!(config.mic_language, Some("en".to_string()));
        assert_eq!(config.monitor_language, Some("de".to_string()));
        assert_eq!(config.whisper_url, "http://localhost:8080");
        assert_eq!(config.vad_threshold, 0.5);
        assert_eq!(config.silence_hold_ms, 500);
        assert_eq!(config.duration_cap_ms, 30000);
        assert_eq!(config.confidence_floor, 0.6);
    }

    #[test]
    fn absent_optional_language_fields_default_to_none() {
        let toml = r#"
            store_root = "/tmp/trai-store"
            mic_source = "alsa_input.default"
            monitor_source = "alsa_output.default.monitor"
            whisper_url = "http://localhost:8080"
            vad_threshold = 0.5
            silence_hold_ms = 500
            duration_cap_ms = 30000
            confidence_floor = 0.6
        "#;

        let config = Config::from_toml_str(toml).unwrap();

        assert_eq!(config.mic_language, None);
        assert_eq!(config.monitor_language, None);
    }

    #[test]
    fn fully_valid_config_is_ok() {
        assert!(Config::from_toml_str(&valid_toml()).is_ok());
    }

    #[test]
    fn missing_required_key_names_that_key_in_the_error() {
        let required_keys = [
            "store_root",
            "mic_source",
            "monitor_source",
            "whisper_url",
            "vad_threshold",
            "silence_hold_ms",
            "duration_cap_ms",
            "confidence_floor",
        ];

        for key in required_keys {
            let toml = remove_line_starting_with(&valid_toml(), key);
            let err = Config::from_toml_str(&toml)
                .expect_err(&format!("expected error when `{key}` is missing"));
            assert!(
                err.to_string().contains(key),
                "error for missing `{key}` did not name it: {err}"
            );
        }
    }

    #[test]
    fn invalid_whisper_url_names_that_key_in_the_error() {
        let toml = valid_toml().replace(
            r#"whisper_url = "http://localhost:8080""#,
            r#"whisper_url = "not a url""#,
        );

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("whisper_url"));
    }

    #[test]
    fn out_of_range_confidence_floor_names_that_key_in_the_error() {
        let toml = valid_toml().replace("confidence_floor = 0.6", "confidence_floor = 1.5");

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("confidence_floor"));
    }

    #[test]
    fn out_of_range_vad_threshold_names_that_key_in_the_error() {
        let toml = valid_toml().replace("vad_threshold = 0.5", "vad_threshold = 1.5");

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("vad_threshold"));
    }

    #[test]
    fn zero_silence_hold_ms_names_that_key_in_the_error() {
        let toml = valid_toml().replace("silence_hold_ms = 500", "silence_hold_ms = 0");

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("silence_hold_ms"));
    }

    #[test]
    fn zero_duration_cap_ms_names_that_key_in_the_error() {
        let toml = valid_toml().replace("duration_cap_ms = 30000", "duration_cap_ms = 0");

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("duration_cap_ms"));
    }

    fn remove_line_starting_with(toml: &str, key: &str) -> String {
        toml.lines()
            .filter(|line| !line.trim_start().starts_with(key))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
