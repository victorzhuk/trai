## 1. Pure core, test-first

- [ ] 1.1 Failing test: a sample stream that goes silence → speech → silence opens a Segment at speech onset and closes it once the silence hold elapses. Acceptance: the Segment's start and end timestamps bracket the speech, and no Segment is produced for the leading silence.
- [ ] 1.2 Implement the segmenter over `earshot` until 1.1 passes.
- [ ] 1.3 Failing test: continuous speech longer than the duration cap force-closes a Segment at the cap and opens the next one immediately. Acceptance: no sample is dropped between the two Segments.
- [ ] 1.4 Extend the segmenter until 1.3 passes.
- [ ] 1.5 Failing test: Segments from two Streams, delivered to the transcript out of order, appear ordered by start timestamp with correct `me`/`them` tags. Acceptance: a reply arriving late inserts at its timestamp position, not at the end.
- [ ] 1.6 Implement transcript ordering until 1.5 passes.
- [ ] 1.7 Failing test: a Segment whose mean word probability is below the configured floor is discarded and never reaches the transcript. Acceptance: a Segment above the floor passes through unchanged.
- [ ] 1.8 Implement the confidence filter until 1.7 passes.
- [ ] 1.9 Failing test: appending Segments then reading them back round-trips the transcript, and a file whose final line is truncated mid-JSON reads back every complete line without error. Acceptance: the truncated line is dropped, the rest survive.
- [ ] 1.10 Implement the append-only store until 1.9 passes.

## 2. Transcription seam, test-first

- [ ] 2.1 Define the `Transcriber` trait and a fake returning canned text after a controllable delay. Acceptance: tests can force out-of-order replies deterministically.
- [ ] 2.2 Failing test: the pipeline driven by the fake produces an ordered, tagged, persisted transcript for a two-Stream conversation. Acceptance: transcript order matches spoken order, not reply order.
- [ ] 2.3 Implement the pipeline core until 2.2 passes.
- [ ] 2.4 Implement the whisper client behind the trait: multipart POST with `verbose_json`, `temperature=0.0`, and `language` when configured. Acceptance: mean word confidence is extracted from the response for the filter in 1.8.

## 3. Capture shell, tests after

- [ ] 3.1 Spawn one `pw-record` per Stream bound to its configured source, emitting 16 kHz mono raw PCM to stdout. Acceptance: both processes exit cleanly when the Recording stops.
- [ ] 3.2 Tee each Stream's samples to its WAV writer and to its segmenter. Acceptance: WAV duration matches Recording duration within a second.
- [ ] 3.3 Finalize both WAV headers on stop. Acceptance: the files play in an external player.

## 4. Configuration

- [ ] 4.1 Load TOML: Store root, microphone and monitor source names, whisper server URL, silence hold, duration cap, confidence floor. Acceptance: every value used by the pipeline comes from config, none hardcoded.
- [ ] 4.2 Fail at startup with an actionable message naming the offending key when config is missing or invalid. Acceptance: a missing Store root does not surface as a panic during the first Segment.

## 5. Window and panel

- [ ] 5.1 Slint window with the original-transcription panel, each row showing speaker tag, timestamp and text, inserted at its timestamp position. Acceptance: a late-arriving line appears in the middle of the panel rather than at the bottom.
- [ ] 5.2 Start and stop controls driving the Recording lifecycle. Acceptance: stop finalizes WAV files and closes the transcript file.

## 6. Verification

- [ ] 6.1 Smoke run against the live whisper server with real speech on both Streams. Acceptance: both `me` and `them` lines appear, and panel content matches `segments.jsonl` line for line.
- [ ] 6.2 Kill the process mid-Recording. Acceptance: `segments.jsonl` reads back cleanly and contains everything spoken up to the kill.
- [ ] 6.3 Record ten minutes of a real meeting and note observed silence-hold and confidence-floor behaviour. Acceptance: tuned values written back into the config file.
