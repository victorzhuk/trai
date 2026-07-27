use crate::domain::{Segment, SpeakerTag};

const PENDING_TRANSLATION: &str = "…";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRow {
    pub speaker: String,
    pub timestamp: String,
    pub text: String,
    pub translation: String,
}

pub fn build_rows(segments: &[Segment]) -> Vec<TranscriptRow> {
    segments
        .iter()
        .map(|segment| TranscriptRow {
            speaker: speaker_label(segment.speaker_tag).into(),
            timestamp: format_timestamp(segment.start_ms),
            text: segment.text.clone(),
            translation: segment
                .translation
                .clone()
                .unwrap_or_else(|| PENDING_TRANSLATION.to_string()),
        })
        .collect()
}

fn speaker_label(tag: SpeakerTag) -> &'static str {
    match tag {
        SpeakerTag::Me => "me",
        SpeakerTag::Them => "them",
    }
}

fn format_timestamp(start_ms: u64) -> String {
    let total_seconds = start_ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

pub fn insert_ordered(transcript: &mut Vec<Segment>, segment: Segment) {
    let position = transcript.partition_point(|existing| existing.start_ms <= segment.start_ms);
    transcript.insert(position, segment);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SpeakerTag;

    fn make_segment(
        id: u64,
        speaker_tag: SpeakerTag,
        start_ms: u64,
        translation: Option<&str>,
    ) -> Segment {
        Segment {
            id,
            speaker_tag,
            start_ms,
            end_ms: start_ms + 100,
            text: format!("segment-{id}"),
            mean_confidence: 0.9,
            translation: translation.map(String::from),
        }
    }

    #[test]
    fn out_of_order_segments_land_sorted_by_start_ms() {
        let mut transcript: Vec<Segment> = Vec::new();

        super::insert_ordered(
            &mut transcript,
            make_segment(1, SpeakerTag::Them, 500, None),
        );
        super::insert_ordered(&mut transcript, make_segment(2, SpeakerTag::Me, 0, None));

        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].start_ms, 0);
        assert_eq!(transcript[0].speaker_tag, SpeakerTag::Me);
        assert_eq!(transcript[1].start_ms, 500);
        assert_eq!(transcript[1].speaker_tag, SpeakerTag::Them);
    }

    #[test]
    fn late_arriving_reply_inserts_at_its_timestamp_not_at_the_end() {
        let mut transcript: Vec<Segment> = Vec::new();

        super::insert_ordered(&mut transcript, make_segment(1, SpeakerTag::Me, 0, None));
        super::insert_ordered(
            &mut transcript,
            make_segment(2, SpeakerTag::Them, 500, None),
        );

        // Arrives last, but its start_ms sits between the first two.
        super::insert_ordered(&mut transcript, make_segment(3, SpeakerTag::Me, 250, None));

        assert_eq!(transcript.len(), 3);
        assert_eq!(transcript[0].id, 1);
        assert_eq!(
            transcript[1].id, 3,
            "the late reply must land in the middle"
        );
        assert_eq!(transcript[2].id, 2);

        assert!(transcript
            .windows(2)
            .all(|w| w[0].start_ms <= w[1].start_ms));
    }

    #[test]
    fn build_rows_renders_pending_and_translated_text() {
        let segments = vec![
            make_segment(1, SpeakerTag::Me, 0, None),
            make_segment(2, SpeakerTag::Them, 2_000, Some("bonjour")),
        ];
        let rows = build_rows(&segments);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].speaker, "me");
        assert_eq!(rows[0].timestamp, "00:00");
        assert_eq!(rows[0].translation, "…");
        assert_eq!(rows[1].translation, "bonjour");
    }

    #[test]
    fn build_rows_preserves_row_indices_when_translation_arrives() {
        let before = vec![
            make_segment(1, SpeakerTag::Them, 0, None),
            make_segment(2, SpeakerTag::Me, 500, None),
            make_segment(3, SpeakerTag::Them, 1_000, None),
        ];
        let mut after = before.clone();
        after[1].translation = Some("trad".into());

        let before_rows = build_rows(&before);
        let after_rows = build_rows(&after);

        assert_eq!(before_rows.len(), after_rows.len());
        assert_eq!(before_rows[0], after_rows[0]);
        assert_eq!(before_rows[2], after_rows[2]);
        assert_eq!(before_rows[1].translation, "…");
        assert_eq!(after_rows[1].translation, "trad");
        assert_ne!(before_rows[1], after_rows[1]);
    }
}
