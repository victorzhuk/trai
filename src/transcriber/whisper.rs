use serde::Deserialize;

use super::{TranscribeError, Transcriber, Transcription};
use crate::segmenter::SAMPLE_RATE_HZ;

pub struct WhisperClient {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl WhisperClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::blocking::Client::new(),
        }
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
        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

        let url = format!("{}/inference", self.base_url);
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .map_err(|e| TranscribeError::new(format!("whisper request failed: {e}")))?;

        let body = response
            .text()
            .map_err(|e| TranscribeError::new(format!("failed to read whisper response: {e}")))?;

        parse_transcription(&body)
    }
}

/// Hand-rolled minimal PCM16 mono WAV container. `hound` isn't a
/// dependency yet (it lands with the capture shell's file writing in
/// a later task); pulling it in here just for an in-memory 44-byte
/// header would be a dependency for a one-off, so this writes the
/// RIFF/WAVE/fmt/data chunks directly instead.
fn samples_to_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;

    let data_len = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * CHANNELS as u32 * (BITS_PER_SAMPLE as u32 / 8);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);

    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[derive(Debug, Default, Deserialize)]
struct WhisperResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    segments: Vec<WhisperSegment>,
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
        text: parsed.text,
        mean_confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
