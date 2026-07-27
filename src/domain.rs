use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerTag {
    Me,
    Them,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: u64,
    pub speaker_tag: SpeakerTag,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub mean_confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    #[serde(default)]
    pub degraded: bool,
    #[serde(skip)]
    pub translation_error: Option<String>,
}
