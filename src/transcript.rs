use crate::domain::Segment;

pub fn insert_ordered(transcript: &mut Vec<Segment>, segment: Segment) {
    let position = transcript.partition_point(|existing| existing.start_ms <= segment.start_ms);
    transcript.insert(position, segment);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SpeakerTag;

    fn make_segment(id: u64, speaker_tag: SpeakerTag, start_ms: u64) -> Segment {
        Segment {
            id,
            speaker_tag,
            start_ms,
            end_ms: start_ms + 100,
            text: format!("segment-{id}"),
            mean_confidence: 0.9,
        }
    }

    #[test]
    fn out_of_order_segments_land_sorted_by_start_ms() {
        let mut transcript: Vec<Segment> = Vec::new();

        super::insert_ordered(&mut transcript, make_segment(1, SpeakerTag::Them, 500));
        super::insert_ordered(&mut transcript, make_segment(2, SpeakerTag::Me, 0));

        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].start_ms, 0);
        assert_eq!(transcript[0].speaker_tag, SpeakerTag::Me);
        assert_eq!(transcript[1].start_ms, 500);
        assert_eq!(transcript[1].speaker_tag, SpeakerTag::Them);
    }

    #[test]
    fn late_arriving_reply_inserts_at_its_timestamp_not_at_the_end() {
        let mut transcript: Vec<Segment> = Vec::new();

        super::insert_ordered(&mut transcript, make_segment(1, SpeakerTag::Me, 0));
        super::insert_ordered(&mut transcript, make_segment(2, SpeakerTag::Them, 500));

        // Arrives last, but its start_ms sits between the first two.
        super::insert_ordered(&mut transcript, make_segment(3, SpeakerTag::Me, 250));

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
}
