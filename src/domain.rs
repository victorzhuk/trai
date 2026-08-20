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

// Ordering primitive the pipeline's persistence and the live view both
// rely on: a Segment lands by start_ms, not arrival order.
pub fn insert_ordered(transcript: &mut Vec<Segment>, segment: Segment) {
    let position = transcript.partition_point(|existing| existing.start_ms <= segment.start_ms);
    transcript.insert(position, segment);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment_with_language(source_language: Option<&str>) -> Segment {
        Segment {
            id: 1,
            speaker_tag: SpeakerTag::Me,
            start_ms: 0,
            end_ms: 100,
            text: "text".to_string(),
            mean_confidence: 0.9,
            source_language: source_language.map(String::from),
            translation: None,
            degraded: false,
            state: SegmentState::Ready,
            translation_error: None,
        }
    }

    #[test]
    fn is_target_language_matches_across_spellings() {
        assert!(segment_with_language(Some("en")).is_target_language("english"));
        assert!(segment_with_language(Some("russian")).is_target_language("ru"));
        assert!(!segment_with_language(Some("de")).is_target_language("en"));
    }

    #[test]
    fn is_target_language_is_false_when_the_source_is_unknown() {
        // A missed translation is worse than a redundant one.
        assert!(!segment_with_language(None).is_target_language("en"));
        assert!(!segment_with_language(Some("klingon")).is_target_language("en"));
    }

    #[test]
    fn insert_ordered_places_segments_by_start_ms_not_arrival() {
        let mut transcript = Vec::new();

        let mut first = segment_with_language(None);
        first.id = 1;
        first.start_ms = 100;
        insert_ordered(&mut transcript, first);

        let mut late = segment_with_language(None);
        late.id = 2;
        late.start_ms = 50;
        insert_ordered(&mut transcript, late);

        assert_eq!(transcript[0].id, 2, "the earlier start_ms lands first");
        assert_eq!(transcript[1].id, 1);
    }
}
