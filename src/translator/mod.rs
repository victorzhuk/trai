pub mod fake;
pub mod openai;

pub use fake::FakeTranslator;
pub use openai::OpenAITranslator;

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct TranslateError(String);

impl TranslateError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for TranslateError {}

pub trait Translator: Send + Sync {
    fn translate(&self, text: &str, context: &[String]) -> Result<String, TranslateError>;
}
