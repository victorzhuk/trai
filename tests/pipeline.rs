use std::sync::Arc;
use std::thread;

use trai::domain::SpeakerTag;
use trai::pipeline::{Pipeline, SegmentInput};
use trai::store::{self, Store};
use trai::transcriber::{FakeTranscriber, Transcription};
use trai::translator::FakeTranslator;

#[test]
fn transcript_lands_in_start_ms_order_even_when_the_later_segment_replies_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("transcript.jsonl");
    let store = Store::open(&path).unwrap();

    let fake = Arc::new(FakeTranscriber::new());
    let translator = Arc::new(FakeTranslator::new());
    let pipeline = Arc::new(Pipeline::new(fake.clone(), translator, store, 0.0));

    let earlier_samples = vec![11i16; 8];
    let later_samples = vec![22i16; 8];

    let earlier_call = fake.expect_call(earlier_samples.clone());
    let later_call = fake.expect_call(later_samples.clone());

    let earlier_input = SegmentInput {
        speaker_tag: SpeakerTag::Them,
        start_ms: 0,
        end_ms: 400,
        samples: earlier_samples,
        language: None,
    };
    let later_input = SegmentInput {
        speaker_tag: SpeakerTag::Me,
        start_ms: 1_000,
        end_ms: 1_400,
        samples: later_samples,
        language: None,
    };

    let pipeline_earlier = pipeline.clone();
    let earlier_handle = thread::spawn(move || pipeline_earlier.submit(earlier_input).unwrap());
    let pipeline_later = pipeline.clone();
    let later_handle = thread::spawn(move || pipeline_later.submit(later_input).unwrap());

    // Release the segment with the LATER start_ms first: its reply
    // arrives before the earlier segment's, but the persisted
    // transcript must still land it second, ordered by start_ms and
    // not by which transcription came back first.
    later_call.respond(Transcription {
        text: "later reply, arrives first".to_string(),
        mean_confidence: 0.95,
    });
    earlier_call.respond(Transcription {
        text: "earlier reply, arrives last".to_string(),
        mean_confidence: 0.9,
    });

    earlier_handle.join().unwrap();
    later_handle.join().unwrap();

    let transcript = store::read_all(&path).unwrap();
    assert_eq!(transcript.len(), 2);

    assert_eq!(transcript[0].start_ms, 0);
    assert_eq!(transcript[0].speaker_tag, SpeakerTag::Them);
    assert_eq!(transcript[0].text, "earlier reply, arrives last");
    assert_eq!(transcript[0].mean_confidence, 0.9);

    assert_eq!(transcript[1].start_ms, 1_000);
    assert_eq!(transcript[1].speaker_tag, SpeakerTag::Me);
    assert_eq!(transcript[1].text, "later reply, arrives first");
    assert_eq!(transcript[1].mean_confidence, 0.95);
}
