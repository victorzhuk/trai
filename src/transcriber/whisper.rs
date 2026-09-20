use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::{TranscribeError, Transcriber, Transcription};
use crate::language;
use crate::segmenter::SAMPLE_RATE_HZ;

/// What the server is asked for when no source language is configured.
/// Omitting the field is not equivalent: whisper's own default is `en`,
/// so an omitted field decodes English rather than detecting.
const DETECT: &str = "auto";

/// Which transcription backend the configured `whisper_url` speaks.
#[derive(Clone, PartialEq)]
pub enum Dialect {
    WhisperCpp,
    OpenAi { api_key: String, model: String },
}

// Manual Debug so the bearer key never lands in logs via {:?}.
impl fmt::Debug for Dialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WhisperCpp => f.debug_struct("WhisperCpp").finish(),
            Self::OpenAi { api_key: _, model } => f
                .debug_struct("OpenAi")
                .field("api_key", &Some::<&str>("<redacted>"))
                .field("model", model)
                .finish(),
        }
    }
}

pub struct WhisperClient {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl WhisperClient {
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self, TranscribeError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| TranscribeError::new(format!("failed to build whisper client: {e}")))?;
        Ok(Self {
            base_url: base_url.into(),
            client,
        })
    }
}

impl Transcriber for WhisperClient {
    fn transcribe(
        &self,
        samples: &[i16],
        language: Option<&str>,
    ) -> Result<Transcription, TranscribeError> {
        let wav = samples_to_wav(samples, SAMPLE_RATE_HZ as u32);
        let part = reqwest::blocking::multipart::Part::bytes(wav)
            .file_name("segment.wav")
            .mime_str("audio/wav")
            .map_err(|e| TranscribeError::new(format!("failed to build multipart part: {e}")))?;

        let form = reqwest::blocking::multipart::Form::new()
            .part("file", part)
            .text("response_format", "verbose_json")
            .text("temperature", "0.0")
            .text("language", language_param(language).to_string());

        let url = format!("{}/inference", self.base_url);
        let began = Instant::now();
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .map_err(|e| TranscribeError::new(format!("whisper request failed: {e}")))?;

        let status = response.status();
        let body = crate::http::read_body(response)
            .map_err(|e| TranscribeError::new(format!("failed to read whisper response: {e}")))?;

        crate::debug!(
            "whisper: {} in {:.2}s for {:.1}s audio, {} bytes",
            status,
            began.elapsed().as_secs_f64(),
            samples.len() as f64 / SAMPLE_RATE_HZ as f64,
            body.len(),
        );

        // Without this a non-2xx reply reaches the JSON parser and surfaces
        // as "invalid whisper response", which points at the wrong thing.
        if !status.is_success() {
            return Err(TranscribeError::new(format!(
                "whisper returned {status}: {}",
                crate::log::preview(&body, 200)
            )));
        }

        parse_transcription(&body)
    }
}

fn language_param(configured: Option<&str>) -> &str {
    match configured {
        Some(language) if !language.trim().is_empty() => language.trim(),
        _ => DETECT,
    }
}

fn samples_to_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .expect("writing WAV headers to memory cannot fail");
        for sample in samples {
            writer
                .write_sample(*sample)
                .expect("writing WAV samples to memory cannot fail");
        }
        writer
            .finalize()
            .expect("finalizing an in-memory WAV cannot fail");
    }
    cursor.into_inner()
}

#[derive(Debug, Default, Deserialize)]
struct WhisperResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    segments: Vec<WhisperSegment>,
    // `language` is what the server decoded with, `detected_language`
    // what it heard, both as English names. `language_probabilities` is
    // keyed by code, and a BTreeMap so a tie resolves the same way
    // every time.
    #[serde(default)]
    language: String,
    #[serde(default)]
    detected_language: String,
    #[serde(default)]
    language_probabilities: BTreeMap<String, f32>,
}

#[derive(Debug, Default, Deserialize)]
struct WhisperSegment {
    #[serde(default)]
    words: Vec<WhisperWord>,
}

#[derive(Debug, Default, Deserialize)]
struct WhisperWord {
    #[serde(default)]
    probability: f32,
}

fn parse_transcription(body: &str) -> Result<Transcription, TranscribeError> {
    let parsed: WhisperResponse = serde_json::from_str(body)
        .map_err(|e| TranscribeError::new(format!("invalid whisper response: {e}")))?;

    let probabilities: Vec<f32> = parsed
        .segments
        .iter()
        .flat_map(|segment| segment.words.iter().map(|word| word.probability))
        .collect();

    let mean_confidence = if probabilities.is_empty() {
        0.0
    } else {
        probabilities.iter().sum::<f32>() / probabilities.len() as f32
    };

    Ok(Transcription {
        language: detected_language(&parsed),
        text: parsed.text,
        mean_confidence,
    })
}

// The probability map already speaks codes, so it is read first; the
// name fields are the fallback for a server that reports only those.
fn detected_language(parsed: &WhisperResponse) -> Option<String> {
    let most_likely = parsed
        .language_probabilities
        .iter()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(code, _)| code.as_str());

    most_likely
        .and_then(language::canonical)
        .or_else(|| language::canonical(&parsed.detected_language))
        .or_else(|| language::canonical(&parsed.language))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn request_to_a_silent_server_times_out_instead_of_hanging() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client =
            WhisperClient::new(format!("http://{address}"), Duration::from_millis(250)).unwrap();

        let began = Instant::now();
        let err = client
            .transcribe(&[0i16; SAMPLE_RATE_HZ as usize], None)
            .expect_err("a server that never answers must fail the request");

        assert!(began.elapsed() < Duration::from_secs(10));
        assert!(err.to_string().contains("whisper request failed"));
        drop(listener);
    }

    #[test]
    fn samples_to_wav_produces_a_parseable_header() {
        let wav = samples_to_wav(&[1i16, -2, 300], 16000);
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16000);
        assert_eq!(reader.spec().bits_per_sample, 16);
        assert_eq!(reader.duration(), 3);
    }

    #[test]
    fn parses_mean_confidence_from_verbose_json_words() {
        let body = r#"{
            "text": " Hello world",
            "segments": [
                {
                    "words": [
                        {"word": " Hello", "probability": 0.9},
                        {"word": " world", "probability": 0.8}
                    ]
                },
                {
                    "words": [
                        {"word": " again", "probability": 1.0}
                    ]
                }
            ]
        }"#;

        let transcription = parse_transcription(body).unwrap();
        assert_eq!(transcription.text, " Hello world");
        assert!((transcription.mean_confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn missing_words_yields_zero_confidence() {
        let body = r#"{"text": "", "segments": []}"#;
        let transcription = parse_transcription(body).unwrap();
        assert_eq!(transcription.mean_confidence, 0.0);
    }

    #[test]
    fn absent_configured_language_asks_the_server_to_detect() {
        assert_eq!(language_param(None), "auto");
        assert_eq!(language_param(Some("  ")), "auto");
        assert_eq!(language_param(Some(" de ")), "de");
    }

    #[test]
    fn most_likely_code_from_the_probability_map_wins() {
        let body = r#"{
            "text": " привет",
            "language": "english",
            "detected_language": "english",
            "language_probabilities": {"en": 0.11, "ru": 0.81, "de": 0.08},
            "segments": []
        }"#;

        let transcription = parse_transcription(body).unwrap();
        assert_eq!(transcription.language.as_deref(), Some("ru"));
    }

    #[test]
    fn language_name_is_used_when_no_probability_map_is_reported() {
        let body = r#"{"text": " hallo", "detected_language": "german", "segments": []}"#;
        let transcription = parse_transcription(body).unwrap();
        assert_eq!(transcription.language.as_deref(), Some("de"));

        let decoded_only = r#"{"text": " hallo", "language": "german", "segments": []}"#;
        let transcription = parse_transcription(decoded_only).unwrap();
        assert_eq!(transcription.language.as_deref(), Some("de"));
    }

    #[test]
    fn reply_without_any_language_field_reports_none() {
        let body = r#"{"text": " hello", "segments": []}"#;
        let transcription = parse_transcription(body).unwrap();
        assert_eq!(transcription.language, None);
    }
}
