use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerTag {
    Me,
    Them,
}

/// Where a Segment stands in the live view. Only `Ready` Segments are
/// ever written to the transcript file: the other two exist to keep a
/// row on screen while whisper is working, or after it refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SegmentState {
    Transcribing,
    #[default]
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: u64,
    pub speaker_tag: SpeakerTag,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub mean_confidence: f32,
    /// ISO 639-1 code of the language `text` is in: the Stream's
    /// configured language, or what the server detected. `None` on
    /// Recordings made before languages were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    #[serde(default)]
    pub degraded: bool,
    #[serde(skip)]
    pub state: SegmentState,
    #[serde(skip)]
    pub translation_error: Option<String>,
}

impl Segment {
    /// True when the Segment is already in `target_language`, so there
    /// is nothing for a Translate backend to do. An unknown source
    /// language is never a match: a missed translation is worse than a
    /// redundant one.
    pub fn is_target_language(&self, target_language: &str) -> bool {
        self.source_language
            .as_deref()
            .is_some_and(|source| crate::language::same(source, target_language))
    }
}
