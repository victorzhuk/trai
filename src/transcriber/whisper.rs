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
    dialect: Dialect,
    client: reqwest::blocking::Client,
}

impl WhisperClient {
    pub fn new(
        base_url: impl Into<String>,
        timeout: Duration,
        dialect: Dialect,
    ) -> Result<Self, TranscribeError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| TranscribeError::new(format!("failed to build whisper client: {e}")))?;
        Ok(Self {
            base_url: base_url.into(),
            dialect,
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

        let mut form = reqwest::blocking::multipart::Form::new()
            .part("file", part)
            .text("response_format", "verbose_json")
            .text("temperature", "0.0");

        let request = match &self.dialect {
            Dialect::WhisperCpp => {
                form = form.text("language", language_param(language).to_string());
                self.client
                    .post(format!("{}/inference", self.base_url))
            }
            Dialect::OpenAi { api_key, model } => {
                // Omitted is not equivalent to `auto`: the cloud dialect
                // detects the language itself when the field is absent.
                if let Some(language) = language.map(str::trim).filter(|l| !l.is_empty()) {
                    form = form.text("language", language.to_string());
                }
                form = form.text("model", model.clone());
                self.client
                    .post(format!("{}/audio/transcriptions", self.base_url))
                    .bearer_auth(api_key)
                }
        };
        let began = Instant::now();
        let response = request
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
    #[serde(default)]
    avg_logprob: Option<f32>,
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

    let mean_confidence = if !probabilities.is_empty() {
        probabilities.iter().sum::<f32>() / probabilities.len() as f32
    } else {
        let logprobs: Vec<f32> = parsed
            .segments
            .iter()
            .filter_map(|segment| segment.avg_logprob)
            .collect();
        if logprobs.is_empty() {
            0.0
        } else {
            (logprobs.iter().sum::<f32>() / logprobs.len() as f32).exp()
        }
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    /// One-shot HTTP double: captures the raw request (headers plus
    /// Content-Length body) and replies with the given status and body.
    fn serve_capture(
        status_line: &str,
        response_body: &str,
    ) -> (String, Arc<Mutex<Vec<u8>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let status_line = status_line.to_string();
        let response_body = response_body.to_string();

        let captured_clone = captured.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            let header_end = loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "client closed before sending headers");
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                {
                    break pos + 4;
                }
            };
            let content_length: usize = String::from_utf8_lossy(&request[..header_end])
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse().ok())
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "client closed before sending the body");
                request.extend_from_slice(&buf[..n]);
            }
            *captured_clone.lock().unwrap() = request;

            let response = format!(
                "{status_line}\r\n\
Content-Type: application/json\r\n\
Content-Length: {}\r\n\
Connection: close\r\n\
\r\n\
{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        (format!("http://{address}"), captured, server)
    }

    #[test]
    fn whisper_cpp_dialect_request_is_unchanged() {
        let (base_url, captured, server) =
            serve_capture("HTTP/1.1 200 OK", r#"{"text":" hello","segments":[]}"#);
        let client =
            WhisperClient::new(base_url, TEST_TIMEOUT, Dialect::WhisperCpp).unwrap();

        let transcription = client.transcribe(&[0i16; 16], None).unwrap();

        assert_eq!(transcription.text, " hello");
        let request = String::from_utf8_lossy(&captured.lock().unwrap()).into_owned();
        assert!(request.starts_with("POST /inference"));
        // hyper lowercases header names on the wire; the value case is kept.
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        assert!(request.contains("name=\"language\"\r\n\r\nauto"));
        assert!(request.contains("name=\"temperature\"\r\n\r\n0.0"));
        assert!(request.contains("name=\"response_format\"\r\n\r\nverbose_json"));
        assert!(request.contains("filename=\"segment.wav\""));
        assert!(!request.contains("name=\"model\""));
        server.join().unwrap();
    }

    #[test]
    fn openai_dialect_posts_to_audio_transcriptions_with_bearer_and_model_and_no_language() {
        let (base_url, captured, server) =
            serve_capture("HTTP/1.1 200 OK", r#"{"text":" hello","segments":[]}"#);
        let client = WhisperClient::new(
            base_url,
            TEST_TIMEOUT,
            Dialect::OpenAi {
                api_key: "test-key".to_string(),
                model: "whisper-large-v3-turbo".to_string(),
            },
        )
        .unwrap();

        let transcription = client.transcribe(&[0i16; 16], None).unwrap();

        assert_eq!(transcription.text, " hello");
        let request = String::from_utf8_lossy(&captured.lock().unwrap()).into_owned();
        assert!(request.starts_with("POST /audio/transcriptions"));
        // hyper lowercases header names on the wire; the value case is kept.
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-key"));
        assert!(request.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo"));
        assert!(request.contains("name=\"response_format\"\r\n\r\nverbose_json"));
        assert!(request.contains("name=\"temperature\"\r\n\r\n0.0"));
        assert!(!request.contains("name=\"language\""));
        // All-zero samples keep the WAV bytes deterministic, so the
        // absence of `auto` anywhere in the request is a sound check.
        assert!(!request.contains("auto"));
        server.join().unwrap();
    }

    #[test]
    fn openai_dialect_with_configured_language_sends_it_and_never_auto() {
        let (base_url, captured, server) =
            serve_capture("HTTP/1.1 200 OK", r#"{"text":" hallo","segments":[]}"#);
        let client = WhisperClient::new(
            base_url,
            TEST_TIMEOUT,
            Dialect::OpenAi {
                api_key: "test-key".to_string(),
                model: "whisper-large-v3-turbo".to_string(),
            },
        )
        .unwrap();

        client.transcribe(&[0i16; 16], Some(" de ")).unwrap();

        let request = String::from_utf8_lossy(&captured.lock().unwrap()).into_owned();
        assert!(request.contains("name=\"language\"\r\n\r\nde"));
        assert!(!request.contains("auto"));
        server.join().unwrap();
    }

    #[test]
    fn non_2xx_cloud_response_surfaces_status_and_preview_without_the_key() {
        let (base_url, _captured, server) = serve_capture(
            "HTTP/1.1 401 Unauthorized",
            r#"{"error":"bad key"}"#,
        );
        let client = WhisperClient::new(
            base_url,
            TEST_TIMEOUT,
            Dialect::OpenAi {
                api_key: "test-key".to_string(),
                model: "whisper-large-v3-turbo".to_string(),
            },
        )
        .unwrap();

        let err = client
            .transcribe(&[0i16; 16], None)
            .expect_err("a 401 must fail the request");

        let message = err.to_string();
        assert!(message.contains("whisper returned 401"));
        assert!(message.contains("bad key"));
        assert!(!message.contains("test-key"));
        // Headers, the only place the key travels, never reach the body
        // preview; checking the Debug form too guards future refactors.
        assert!(!format!("{err:?}").contains("test-key"));
        server.join().unwrap();
    }

    #[test]
    fn request_to_a_silent_server_times_out_instead_of_hanging() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client =
            WhisperClient::new(
                format!("http://{address}"),
                Duration::from_millis(250),
                Dialect::WhisperCpp,
            )
            .unwrap();

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
    fn words_take_precedence_over_avg_logprob() {
        let body = r#"{
            "text": " Hello world",
            "segments": [
                {
                    "words": [
                        {"word": " Hello", "probability": 0.9},
                        {"word": " world", "probability": 0.8}
                    ]
                },
                {"avg_logprob": -0.8}
            ]
        }"#;

        let transcription = parse_transcription(body).unwrap();
        assert!((transcription.mean_confidence - 0.85).abs() < 1e-6);
    }

    #[test]
    fn segments_with_only_avg_logprob_yield_geometric_mean_confidence() {
        let body = r#"{
            "text": " Hello world",
            "segments": [
                {"avg_logprob": -0.3},
                {"avg_logprob": -0.5}
            ]
        }"#;

        let transcription = parse_transcription(body).unwrap();
        assert!((transcription.mean_confidence - 0.6703).abs() < 1e-3);
        assert!(transcription.mean_confidence >= 0.6);
    }

    #[test]
    fn hallucination_level_avg_logprob_lands_below_the_floor() {
        let body = r#"{"text": " Hello", "segments": [{"avg_logprob": -0.8}]}"#;

        let transcription = parse_transcription(body).unwrap();
        assert!((transcription.mean_confidence - 0.4493).abs() < 1e-3);
        assert!(transcription.mean_confidence < 0.6);
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
