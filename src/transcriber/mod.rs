pub mod fake;
pub mod whisper;

pub use fake::FakeTranscriber;
pub use whisper::WhisperClient;

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub text: String,
    pub mean_confidence: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscribeError(String);

impl TranscribeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for TranscribeError {}

pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        samples: &[i16],
        language: Option<&str>,
    ) -> Result<Transcription, TranscribeError>;
}
