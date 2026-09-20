// Test doubles compile only for unit tests or the integration-test
// build (the `test-doubles` feature the dev-dependency enables), not
// into the production binary.
#[cfg(any(test, feature = "test-doubles"))]
pub mod fake;
pub mod whisper;

#[cfg(any(test, feature = "test-doubles"))]
pub use fake::FakeTranscriber;
pub use whisper::{Dialect, WhisperClient};

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub text: String,
    pub mean_confidence: f32,
    /// Language the server reported detecting, as an ISO 639-1 code.
    /// `None` when the server reported nothing recognisable.
    pub language: Option<String>,
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
