pub const FRAME_LEN: usize = 256;
pub const SAMPLE_RATE_HZ: u64 = 16_000;

pub fn to_ms(sample: u64) -> u64 {
    sample * 1000 / SAMPLE_RATE_HZ
}

fn ms_to_samples(ms: u64) -> u64 {
    ms * SAMPLE_RATE_HZ / 1000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechSpan {
    pub start_sample: u64,
    pub end_sample: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentEvent {
    Opened { start_sample: u64 },
    Closed(SpeechSpan),
}

pub struct Segmenter {
    detector: earshot::Detector,
    threshold: f32,
    silence_hold_samples: u64,
    duration_cap_samples: u64,
    live_chunk_samples: u64,
    buffer: Vec<i16>,
    total_samples: u64,
    in_speech: bool,
    segment_start: u64,
    silence_run: u64,
    last_voice_end: u64,
}

impl Segmenter {
    pub fn new(
        threshold: f32,
        silence_hold_ms: u64,
        duration_cap_ms: u64,
        live_chunk_ms: u64,
    ) -> Self {
        Self {
            detector: earshot::Detector::default(),
            threshold,
            silence_hold_samples: ms_to_samples(silence_hold_ms),
            duration_cap_samples: ms_to_samples(duration_cap_ms),
            live_chunk_samples: ms_to_samples(live_chunk_ms),
            buffer: Vec::new(),
            total_samples: 0,
            in_speech: false,
            segment_start: 0,
            silence_run: 0,
            last_voice_end: 0,
        }
    }

    /// Accepts samples of any length; buffers internally until full
    /// 256-sample frames are available. Returns, in order, every
    /// Opened/Closed event produced while processing this chunk. A
    /// duration-cap force-close yields both a Closed and an Opened
    /// event for the same frame, in that order.
    pub fn push_samples(&mut self, samples: &[i16]) -> Vec<SegmentEvent> {
        self.buffer.extend_from_slice(samples);
        let mut events = Vec::new();
        // One drain per call, not per frame: this runs at 62.5 fps per
        // stream, so per-frame drain would alloc and memmove per frame.
        let ready = self.buffer.len() / FRAME_LEN * FRAME_LEN;
        let chunk: Vec<i16> = self.buffer.drain(..ready).collect();
        for frame in chunk.chunks_exact(FRAME_LEN) {
            self.process_frame(frame, &mut events);
        }
        events
    }

    /// Call once the stream has ended. Closes a still-open Segment
    /// using the last voiced frame's end as its boundary.
    pub fn finish(self) -> Option<SpeechSpan> {
        if self.in_speech {
            Some(SpeechSpan {
                start_sample: self.segment_start,
                end_sample: self.last_voice_end.max(self.segment_start),
            })
        } else {
            None
        }
    }

    fn process_frame(&mut self, frame: &[i16], events: &mut Vec<SegmentEvent>) {
        let frame_start = self.total_samples;
        let score = self.detector.predict_i16(frame);
        self.total_samples += FRAME_LEN as u64;
        let frame_end = self.total_samples;
        let is_voice = score > self.threshold;

        if !self.in_speech {
            if is_voice {
                self.in_speech = true;
                self.segment_start = frame_start;
                self.silence_run = 0;
                self.last_voice_end = frame_end;
                events.push(SegmentEvent::Opened {
                    start_sample: frame_start,
                });
            }
            return;
        }

        if is_voice {
            self.silence_run = 0;
            self.last_voice_end = frame_end;
        } else {
            self.silence_run += FRAME_LEN as u64;
            if self.silence_run >= self.silence_hold_samples {
                let span = SpeechSpan {
                    start_sample: self.segment_start,
                    end_sample: self.last_voice_end,
                };
                self.in_speech = false;
                self.silence_run = 0;
                events.push(SegmentEvent::Closed(span));
                return;
            }
        }

        // Mid-speech chunking: cut ongoing speech at whichever cap fires
        // first. `live_chunk_ms` is the normal live-transcription interval
        // (short, so text appears while the speaker is still talking);
        // `duration_cap_ms` is a hard safety ceiling for when live_chunk
        // is set higher or equal.
        let chunk_samples = if self.live_chunk_samples < self.duration_cap_samples {
            self.live_chunk_samples
        } else {
            self.duration_cap_samples
        };

        if frame_end - self.segment_start >= chunk_samples {
            let span = SpeechSpan {
                start_sample: self.segment_start,
                end_sample: frame_end,
            };
            self.segment_start = frame_end;
            self.silence_run = 0;
            self.last_voice_end = frame_end;
            events.push(SegmentEvent::Closed(span));
            events.push(SegmentEvent::Opened {
                start_sample: frame_end,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_speech_frame(len: usize, phase_start: usize) -> Vec<i16> {
        let formants = [180.0, 420.0, 900.0, 1800.0, 2600.0];
        (phase_start..phase_start + len)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE_HZ as f32;
                let mut sample = 0.0f32;
                for f in formants {
                    sample += (2.0 * std::f32::consts::PI * f * t).sin();
                }
                (sample / formants.len() as f32 * 22000.0) as i16
            })
            .collect()
    }

    #[test]
    fn synthetic_speech_frame_scores_above_threshold() {
        let mut detector = earshot::Detector::default();
        let frame = synthetic_speech_frame(FRAME_LEN, 0);
        let score = detector.predict_i16(&frame);
        assert!(score > 0.5, "expected voice score > 0.5, got {score}");
    }

    #[test]
    fn silence_speech_silence_opens_and_closes_segment() {
        let lead_silence_len = FRAME_LEN * 10;
        let speech_len = FRAME_LEN * 20;
        let trail_silence_len = FRAME_LEN * 40;

        let silence_lead = vec![0i16; lead_silence_len];
        let speech = synthetic_speech_frame(speech_len, lead_silence_len);
        let silence_trail = vec![0i16; trail_silence_len];

        let mut segmenter = Segmenter::new(0.5, 200, 20_000, 20_000);

        let mut events = Vec::new();
        events.extend(segmenter.push_samples(&silence_lead[..1000]));
        events.extend(segmenter.push_samples(&silence_lead[1000..]));
        events.extend(segmenter.push_samples(&speech[..777]));
        events.extend(segmenter.push_samples(&speech[777..]));
        events.extend(segmenter.push_samples(&silence_trail));

        let opened: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                SegmentEvent::Opened { start_sample } => Some(*start_sample),
                _ => None,
            })
            .collect();
        let spans: Vec<SpeechSpan> = events
            .into_iter()
            .filter_map(|e| match e {
                SegmentEvent::Closed(span) => Some(span),
                _ => None,
            })
            .collect();

        assert_eq!(
            opened.len(),
            1,
            "expected exactly one Opened event, got {}",
            opened.len()
        );
        assert_eq!(
            spans.len(),
            1,
            "expected exactly one speech segment, got {}",
            spans.len()
        );
        let span = spans[0];
        assert_eq!(
            opened[0], span.start_sample,
            "Opened event's start_sample must match the span it eventually closes"
        );

        assert!(
            span.start_sample >= lead_silence_len as u64,
            "segment must not start within leading silence"
        );
        assert!(
            span.start_sample < (lead_silence_len + speech_len) as u64,
            "segment must start within the speech region"
        );
        assert!(
            span.end_sample > span.start_sample,
            "segment must have positive duration"
        );
        // Allow a short tail past the synthetic speech region: the detector's
        // internal energy smoothing keeps scoring a few frames as voice right
        // after the tone stops, before silence_hold_ms actually closes the
        // Segment. The real acceptance bar is that it closes well within the
        // trailing silence, not that it tracks the tone's exact sample length.
        assert!(
            span.end_sample < (lead_silence_len + speech_len + FRAME_LEN * 8) as u64,
            "segment must close shortly after speech ends, not run into deep trailing silence"
        );

        assert!(
            segmenter.finish().is_none(),
            "segment already closed by silence hold, finish() should have nothing left"
        );
    }

    #[test]
    fn duration_cap_force_closes_without_dropping_samples() {
        let speech_len = FRAME_LEN * 200;
        let speech = synthetic_speech_frame(speech_len, 0);

        let mut segmenter = Segmenter::new(0.5, 200, 500, 500);
        let events = segmenter.push_samples(&speech);

        let spans: Vec<SpeechSpan> = events
            .iter()
            .filter_map(|e| match e {
                SegmentEvent::Closed(span) => Some(*span),
                _ => None,
            })
            .collect();

        assert!(
            spans.len() >= 2,
            "expected at least two duration-cap-forced segments, got {}",
            spans.len()
        );
        assert_eq!(spans[0].start_sample, 0);
        assert!(spans[0].end_sample > spans[0].start_sample);
        assert_eq!(
            spans[0].end_sample, spans[1].start_sample,
            "no sample must be dropped or duplicated between forced segments"
        );
        assert_eq!(
            spans[1].end_sample - spans[1].start_sample,
            spans[0].end_sample - spans[0].start_sample,
            "consecutive forced segments should have identical duration (the cap)"
        );

        let first_closed_idx = events
            .iter()
            .position(|e| matches!(e, SegmentEvent::Closed(s) if *s == spans[0]))
            .expect("first forced span must appear among the events");
        assert_eq!(
            events.get(first_closed_idx + 1),
            Some(&SegmentEvent::Opened {
                start_sample: spans[0].end_sample
            }),
            "a duration-cap close must be immediately followed by a reopen at the same boundary sample, in that order"
        );
    }

    #[test]
    fn live_chunk_cuts_ongoing_speech_before_the_duration_cap() {
        let speech_len = FRAME_LEN * 200;
        let speech = synthetic_speech_frame(speech_len, 0);

        // live_chunk_ms = 300 (shorter than duration_cap_ms = 500)
        let mut segmenter = Segmenter::new(0.5, 200, 500, 300);
        let events = segmenter.push_samples(&speech);

        let spans: Vec<SpeechSpan> = events
            .iter()
            .filter_map(|e| match e {
                SegmentEvent::Closed(span) => Some(*span),
                _ => None,
            })
            .collect();

        assert!(
            spans.len() >= 3,
            "expected at least three live-chunk segments from 200 frames, got {}",
            spans.len()
        );

        let chunk_samples = ms_to_samples(300);
        for span in &spans {
            let duration = span.end_sample - span.start_sample;
            assert!(
                duration >= chunk_samples
                    && duration < chunk_samples + FRAME_LEN as u64,
                "each live-chunk segment must be approximately live_chunk_ms long (within one frame), got {} samples",
                duration
            );
        }

        // No sample dropped or duplicated between consecutive chunks.
        for w in spans.windows(2) {
            assert_eq!(w[0].end_sample, w[1].start_sample);
        }
    }
}
