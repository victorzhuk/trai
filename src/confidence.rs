use crate::domain::Segment;

pub fn passes_floor(segment: &Segment, floor: f32) -> bool {
    segment.mean_confidence >= floor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SegmentState, SpeakerTag};

    fn make_segment(mean_confidence: f32) -> Segment {
        Segment {
            id: 1,
            speaker_tag: SpeakerTag::Me,
            start_ms: 0,
            end_ms: 100,
            text: "hello".to_string(),
            mean_confidence,
            source_language: None,
            translation: None,
            degraded: false,
            state: SegmentState::Ready,
            translation_error: None,
        }
    }

    #[test]
    fn passes_floor_cases() {
        let floor = 0.6;
        let cases = [
            (0.3, false, "well below floor"),
            (0.59, false, "just below floor"),
            (0.6, true, "exactly at floor"),
            (0.9, true, "above floor"),
        ];

        for (confidence, expected, label) in cases {
            let segment = make_segment(confidence);
            assert_eq!(
                super::passes_floor(&segment, floor),
                expected,
                "case failed: {label} (confidence={confidence}, floor={floor})"
            );
        }
    }
}
