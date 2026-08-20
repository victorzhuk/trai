use crate::domain::{Segment, SegmentState, SpeakerTag};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginalText {
    Transcribing,
    Failed(String),
    Ready(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationText {
    Pending,
    Ready { text: String, degraded: bool },
    Verbatim(String),
    Failed(String),
    // The transcription failed, so there is nothing to translate.
    Absent,
}

// One panel row as two small state machines instead of seven booleans
// whose invalid combinations (transcribing AND text_failed) were
// representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRow {
    pub speaker: String,
    pub mine: bool,
    pub timestamp: String,
    pub original: OriginalText,
    pub translation: TranslationText,
}

/// Snapshot of what the pipeline is busy with, read off the same
/// live-view rows the panels are built from rather than reported
/// through a second channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptStatus {
    pub transcribing: usize,
    pub translating: usize,
    pub failed: usize,
    pub mic_language: Option<String>,
    pub monitor_language: Option<String>,
    pub degraded: bool,
}

pub fn build_row(segment: &Segment, target_language: &str) -> TranscriptRow {
    let original = match &segment.state {
        SegmentState::Transcribing => OriginalText::Transcribing,
        SegmentState::Failed(message) => OriginalText::Failed(message.clone()),
        SegmentState::Ready => OriginalText::Ready(segment.text.clone()),
    };

    let translation = match &segment.state {
        SegmentState::Transcribing => TranslationText::Pending,
        SegmentState::Failed(_) => TranslationText::Absent,
        SegmentState::Ready => {
            if let Some(message) = &segment.translation_error {
                TranslationText::Failed(message.clone())
            } else if let Some(translation) = &segment.translation {
                TranslationText::Ready {
                    text: translation.clone(),
                    degraded: segment.degraded,
                }
            } else if segment.is_target_language(target_language) {
                TranslationText::Verbatim(segment.text.clone())
            } else {
                TranslationText::Pending
            }
        }
    };

    TranscriptRow {
        speaker: speaker_label(segment.speaker_tag).into(),
        mine: segment.speaker_tag == SpeakerTag::Me,
        timestamp: format_timestamp(segment.start_ms),
        original,
        translation,
    }
}

pub fn build_rows(segments: &[Segment], target_language: &str) -> Vec<TranscriptRow> {
    segments
        .iter()
        .map(|segment| build_row(segment, target_language))
        .collect()
}

pub fn summarize(segments: &[Segment], target_language: &str) -> TranscriptStatus {
    let mut status = TranscriptStatus::default();

    for segment in segments {
        match &segment.state {
            SegmentState::Transcribing => status.transcribing += 1,
            SegmentState::Failed(_) => status.failed += 1,
            SegmentState::Ready => {
                if segment.translation.is_none()
                    && segment.translation_error.is_none()
                    && !segment.is_target_language(target_language)
                {
                    status.translating += 1;
                }
                if segment.translation.is_some() {
                    status.degraded = segment.degraded;
                }
            }
        }

        // Segments arrive sorted by start_ms, so the last one carrying a
        // language is the Stream's current one.
        if let Some(language) = &segment.source_language {
            match segment.speaker_tag {
                SpeakerTag::Me => status.mic_language = Some(language.clone()),
                SpeakerTag::Them => status.monitor_language = Some(language.clone()),
            }
        }
    }

    status
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
            source_language: None,
            translation: translation.map(String::from),
            degraded: false,
            state: SegmentState::Ready,
            translation_error: None,
        }
    }

    #[test]
    fn out_of_order_segments_land_sorted_by_start_ms() {
        let mut transcript: Vec<Segment> = Vec::new();

        crate::domain::insert_ordered(
            &mut transcript,
            make_segment(1, SpeakerTag::Them, 500, None),
        );
        crate::domain::insert_ordered(&mut transcript, make_segment(2, SpeakerTag::Me, 0, None));

        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].start_ms, 0);
        assert_eq!(transcript[0].speaker_tag, SpeakerTag::Me);
        assert_eq!(transcript[1].start_ms, 500);
        assert_eq!(transcript[1].speaker_tag, SpeakerTag::Them);
    }

    #[test]
    fn late_arriving_reply_inserts_at_its_timestamp_not_at_the_end() {
        let mut transcript: Vec<Segment> = Vec::new();

        crate::domain::insert_ordered(&mut transcript, make_segment(1, SpeakerTag::Me, 0, None));
        crate::domain::insert_ordered(
            &mut transcript,
            make_segment(2, SpeakerTag::Them, 500, None),
        );

        // Arrives last, but its start_ms sits between the first two.
        crate::domain::insert_ordered(&mut transcript, make_segment(3, SpeakerTag::Me, 250, None));

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
        let rows = build_rows(&segments, "en");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].speaker, "me");
        assert!(rows[0].mine);
        assert_eq!(rows[0].timestamp, "00:00");
        assert_eq!(rows[0].translation, TranslationText::Pending);
        assert_eq!(rows[1].speaker, "them");
        assert!(!rows[1].mine);
        assert_eq!(
            rows[1].translation,
            TranslationText::Ready {
                text: "bonjour".to_string(),
                degraded: false,
            }
        );
    }

    #[test]
    fn build_rows_marks_degraded_segment_and_leaves_other_fields_untouched() {
        let normal = make_segment(1, SpeakerTag::Me, 0, Some("bonjour"));
        let degraded = Segment {
            degraded: true,
            ..normal.clone()
        };

        let normal_rows = build_rows(&[normal], "en");
        let degraded_rows = build_rows(&[degraded], "en");

        assert_eq!(
            degraded_rows[0].translation,
            TranslationText::Ready {
                text: "bonjour".to_string(),
                degraded: true,
            }
        );

        assert_eq!(normal_rows[0].speaker, degraded_rows[0].speaker);
        assert_eq!(normal_rows[0].timestamp, degraded_rows[0].timestamp);
        assert_eq!(normal_rows[0].original, degraded_rows[0].original);
    }

    #[test]
    fn build_rows_surfaces_translation_error_and_leaves_other_fields_untouched() {
        let normal = make_segment(1, SpeakerTag::Me, 0, None);
        let errored = Segment {
            translation_error: Some("model 'x' not found".to_string()),
            ..normal.clone()
        };

        let normal_rows = build_rows(&[normal], "en");
        let error_rows = build_rows(&[errored], "en");

        assert_eq!(
            error_rows[0].translation,
            TranslationText::Failed("model 'x' not found".to_string())
        );

        assert_eq!(normal_rows[0].speaker, error_rows[0].speaker);
        assert_eq!(normal_rows[0].timestamp, error_rows[0].timestamp);
        assert_eq!(normal_rows[0].original, error_rows[0].original);
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

        let before_rows = build_rows(&before, "en");
        let after_rows = build_rows(&after, "en");

        assert_eq!(before_rows.len(), after_rows.len());
        assert_eq!(before_rows[0], after_rows[0]);
        assert_eq!(before_rows[2], after_rows[2]);
        assert_eq!(before_rows[1].translation, TranslationText::Pending);
        assert_eq!(
            after_rows[1].translation,
            TranslationText::Ready {
                text: "trad".to_string(),
                degraded: false,
            }
        );
        assert_ne!(before_rows[1], after_rows[1]);
    }

    #[test]
    fn build_rows_shows_a_segment_already_in_the_target_language_verbatim() {
        let mut segment = make_segment(1, SpeakerTag::Them, 0, None);
        segment.text = "уже по-русски".to_string();
        segment.source_language = Some("ru".to_string());

        let rows = build_rows(&[segment.clone()], "ru");

        assert_eq!(
            rows[0].translation,
            TranslationText::Verbatim("уже по-русски".to_string())
        );

        // The same Segment against another target language is an
        // ordinary line still waiting for its translation.
        let rows = build_rows(&[segment], "en");
        assert_eq!(rows[0].translation, TranslationText::Pending);
    }

    #[test]
    fn build_rows_translates_a_segment_whose_language_is_unknown() {
        let mut segment = make_segment(1, SpeakerTag::Me, 0, None);
        segment.source_language = Some("klingon".to_string());

        let rows = build_rows(&[segment], "klingon");

        assert_eq!(rows[0].translation, TranslationText::Pending);
    }

    #[test]
    fn build_rows_marks_an_in_flight_row_and_a_failed_one() {
        let mut transcribing = make_segment(1, SpeakerTag::Me, 0, None);
        transcribing.text = String::new();
        transcribing.state = SegmentState::Transcribing;

        let mut failed = make_segment(2, SpeakerTag::Them, 500, None);
        failed.state = SegmentState::Failed("whisper returned 503".to_string());

        let rows = build_rows(&[transcribing, failed], "en");

        assert_eq!(rows[0].original, OriginalText::Transcribing);
        assert_eq!(rows[0].translation, TranslationText::Pending);

        assert_eq!(
            rows[1].original,
            OriginalText::Failed("whisper returned 503".to_string())
        );
        assert_eq!(rows[1].translation, TranslationText::Absent);
    }

    #[test]
    fn summarize_counts_in_flight_work_and_reports_the_latest_language_per_stream() {
        let mut transcribing = make_segment(1, SpeakerTag::Me, 0, None);
        transcribing.state = SegmentState::Transcribing;

        let mut mic_translated = make_segment(2, SpeakerTag::Me, 500, Some("привет"));
        mic_translated.source_language = Some("en".to_string());
        mic_translated.degraded = true;

        let mut monitor_pending = make_segment(3, SpeakerTag::Them, 1_000, None);
        monitor_pending.source_language = Some("de".to_string());

        let mut monitor_verbatim = make_segment(4, SpeakerTag::Them, 1_500, None);
        monitor_verbatim.source_language = Some("ru".to_string());

        let mut failed = make_segment(5, SpeakerTag::Me, 2_000, None);
        failed.state = SegmentState::Failed("whisper unreachable".to_string());

        let status = summarize(
            &[
                transcribing,
                mic_translated,
                monitor_pending,
                monitor_verbatim,
                failed,
            ],
            "ru",
        );

        assert_eq!(status.transcribing, 1);
        assert_eq!(
            status.translating, 1,
            "only the monitor Segment that is neither translated nor same-language is in flight"
        );
        assert_eq!(status.failed, 1);
        assert_eq!(status.mic_language.as_deref(), Some("en"));
        assert_eq!(status.monitor_language.as_deref(), Some("ru"));
        assert!(status.degraded);
    }

    #[test]
    fn summarize_of_an_empty_transcript_reports_nothing_in_flight() {
        assert_eq!(summarize(&[], "en"), TranscriptStatus::default());
    }
}
