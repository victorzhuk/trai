pub mod fake;
pub mod fallback;
pub mod openai;

pub use fake::FakeTranslator;
pub use fallback::FallbackTranslator;
pub use openai::OpenAITranslator;

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslateErrorKind {
    Unavailable,
    Misconfigured,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranslateError {
    kind: TranslateErrorKind,
    message: String,
}

impl TranslateError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: TranslateErrorKind::Unavailable,
            message: message.into(),
        }
    }

    pub fn misconfigured(message: impl Into<String>) -> Self {
        Self {
            kind: TranslateErrorKind::Misconfigured,
            message: message.into(),
        }
    }

    pub fn new(message: impl Into<String>) -> Self {
        Self::unavailable(message)
    }

    pub fn kind(&self) -> TranslateErrorKind {
        self.kind
    }

    pub fn is_misconfigured(&self) -> bool {
        self.kind == TranslateErrorKind::Misconfigured
    }
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TranslateError {}

#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    pub text: String,
    pub degraded: bool,
}

pub trait Translator: Send + Sync {
    fn translate(&self, text: &str, context: &[String]) -> Result<Translation, TranslateError>;
    fn probe(&self) -> Result<(), TranslateError>;
}
