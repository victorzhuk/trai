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
    pub target_language: String,
    pub translate: TranslateConfig,
    pub vad_threshold: f32,
    pub silence_hold_ms: u64,
    pub duration_cap_ms: u64,
    pub confidence_floor: f32,
}

#[derive(Debug, Clone)]
pub struct TranslateConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
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
            Self::Io(e) => write!(f, "config io error: {e}"),
            Self::Parse(e) => write!(f, "config parse error: {e}"),
            Self::Missing(key) => write!(f, "missing required config key: {key}"),
            Self::Invalid { key, reason } => write!(f, "{key}: {reason}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(error: toml::de::Error) -> Self {
        Self::Parse(error)
    }
}

fn invalid(key: &'static str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key,
        reason: reason.into(),
    }
}

fn require_non_blank(key: &'static str, value: String) -> Result<String, ConfigError> {
    if value.trim().is_empty() {
        Err(invalid(key, "must not be blank"))
    } else {
        Ok(value)
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
    target_language: Option<String>,
    translate: Option<RawTranslateConfig>,
    vad_threshold: Option<f32>,
    silence_hold_ms: Option<u64>,
    duration_cap_ms: Option<u64>,
    confidence_floor: Option<f32>,
}

#[derive(Deserialize)]
struct RawTranslateConfig {
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
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
        let target_language = require_non_blank(
            "target_language",
            raw.target_language
                .ok_or(ConfigError::Missing("target_language"))?,
        )?;
        let translate = raw.translate.ok_or(ConfigError::Missing("translate"))?;
        let translate_base_url = require_non_blank(
            "translate.base_url",
            translate
                .base_url
                .ok_or(ConfigError::Missing("translate.base_url"))?,
        )?;
        let translate_model = require_non_blank(
            "translate.model",
            translate
                .model
                .ok_or(ConfigError::Missing("translate.model"))?,
        )?;
        let translate_api_key = translate.api_key.and_then(|api_key| {
            if api_key.trim().is_empty() {
                None
            } else {
                Some(api_key)
            }
        });
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
        reqwest::Url::parse(&translate_base_url)
            .map_err(|e| invalid("translate.base_url", format!("not a valid URL: {e}")))?;

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
            target_language,
            translate: TranslateConfig {
                base_url: translate_base_url,
                model: translate_model,
                api_key: translate_api_key,
            },
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
            target_language = "en"
            vad_threshold = 0.5
            silence_hold_ms = 500
            duration_cap_ms = 30000
            confidence_floor = 0.6

            [translate]
            base_url = "http://localhost:11434"
            model = "gpt-4o-mini"
            api_key = "not-a-secret"
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
        assert_eq!(config.target_language, "en");
        assert_eq!(config.translate.base_url, "http://localhost:11434");
        assert_eq!(config.translate.model, "gpt-4o-mini");
        assert_eq!(config.translate.api_key.as_deref(), Some("not-a-secret"));
    }

    #[test]
    fn blank_translate_api_key_is_none() {
        let toml = valid_toml().replace(r#"api_key = "not-a-secret""#, r#"api_key = "   ""#);
        let config = Config::from_toml_str(&toml).unwrap();

        assert_eq!(config.translate.api_key, None);
    }

    #[test]
    fn absent_optional_language_and_api_key_fields_default_to_none() {
        let toml = r#"
            store_root = "/tmp/trai-store"
            mic_source = "alsa_input.default"
            monitor_source = "alsa_output.default.monitor"
            whisper_url = "http://localhost:8080"
            target_language = "en"
            vad_threshold = 0.5
            silence_hold_ms = 500
            duration_cap_ms = 30000
            confidence_floor = 0.6

            [translate]
            base_url = "http://localhost:11434"
            model = "gpt-4o-mini"
        "#;

        let config = Config::from_toml_str(toml).unwrap();

        assert_eq!(config.mic_language, None);
        assert_eq!(config.monitor_language, None);
        assert_eq!(config.translate.api_key, None);
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
            "target_language",
            "translate",
            "translate.base_url",
            "translate.model",
            "vad_threshold",
            "silence_hold_ms",
            "duration_cap_ms",
            "confidence_floor",
        ];

        for key in required_keys {
            let toml = remove_required_key(&valid_toml(), key);
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
    fn invalid_translate_base_url_names_that_key_in_the_error() {
        let toml = valid_toml().replace(
            r#"base_url = "http://localhost:11434""#,
            r#"base_url = "not a url""#,
        );

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("translate.base_url"));
    }
    #[test]
    fn invalid_target_language_names_that_key_in_the_error() {
        let toml = valid_toml().replace(r#"target_language = "en""#, r#"target_language = "   ""#);

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("target_language"));
    }

    #[test]
    fn invalid_translate_base_url_names_that_key_in_the_error_when_blank() {
        let toml = valid_toml().replace(
            r#"base_url = "http://localhost:11434""#,
            r#"base_url = "   ""#,
        );

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("translate.base_url"));
    }

    #[test]
    fn invalid_translate_model_names_that_key_in_the_error_when_blank() {
        let toml = valid_toml().replace(r#"model = "gpt-4o-mini""#, r#"model = "   ""#);

        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(err.to_string().contains("translate.model"));
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

    fn remove_required_key(toml: &str, key: &str) -> String {
        if !key.contains('.') && toml.lines().any(|line| line.trim() == format!("[{key}]")) {
            let mut in_table = false;
            let header = format!("[{key}]");
            return toml
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if trimmed == header {
                        in_table = true;
                        return false;
                    }
                    if in_table && trimmed.starts_with('[') {
                        in_table = false;
                    }
                    !in_table
                })
                .collect::<Vec<_>>()
                .join("\n");
        }

        if let Some((parent, child)) = key.split_once('.') {
            let mut in_parent = false;
            let header = format!("[{parent}]");
            toml.lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if trimmed == header {
                        in_parent = true;
                        return true;
                    }
                    if trimmed.starts_with('[') {
                        in_parent = false;
                    }
                    !in_parent
                        || !trimmed.starts_with(&format!("{child} "))
                            && !trimmed.starts_with(&format!("{child}="))
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            remove_line_starting_with(toml, key)
        }
    }
}
