## 1. Translation core, test-first

- [ ] 1.1 Define the `Translator` trait and a fake whose latency and failure are controllable. Acceptance: tests can hold a translation open indefinitely.
- [ ] 1.2 Failing test: translating a Segment sends its text plus the previous three translated lines as context, and no more than three. Acceptance: the fourth-oldest line is absent from the request.
- [ ] 1.3 Implement the rolling context window until 1.2 passes.
- [ ] 1.4 Failing test: a translation record appended after its Segment merges on read into one line carrying both texts. Acceptance: reading a file where the translation record is missing yields the Segment with no translation rather than an error.
- [ ] 1.5 Implement the two-record transcript format and its merge on read until 1.4 passes.

## 2. Translate backend client

- [ ] 2.1 Implement an OpenAI-compatible chat client behind the `Translator` trait, taking base URL, model and optional API key from configuration. Acceptance: the target language comes from config and appears in the request.
- [ ] 2.2 Verify against the configured backend that a Russian and an English Segment each return a usable translation. Acceptance: recorded in the change before marking done.

## 3. Configuration

- [ ] 3.1 Extend the TOML file with the target language and the translate backend definition. Acceptance: startup validation names a missing target language rather than translating into nothing.

## 4. Second panel, tests after

- [ ] 4.1 Add the translation panel beside the original panel. Acceptance: both panels are visible in one window without horizontal scrolling at the default window size.
- [ ] 4.2 Allocate a row in both panels when a Segment closes, the right one in a pending state. Acceptance: row indices stay aligned when a translation is slower than the next Segment.
- [ ] 4.3 Replace the pending row in place when the translation returns. Acceptance: no row is appended or reordered by the arrival of a translation.
- [ ] 4.4 Lock scrolling between the panels. Acceptance: scrolling either panel moves the other to the same row.

## 5. Verification

- [ ] 5.1 Smoke run with real speech on both Streams. Acceptance: every original line acquires its translation, and `segments.jsonl` merges to the same content shown on screen.
- [ ] 5.2 Hold the translate backend unreachable for one Segment during a smoke run. Acceptance: the original panel is unaffected and the placeholder is visibly pending, confirming the failure is contained to one row.
