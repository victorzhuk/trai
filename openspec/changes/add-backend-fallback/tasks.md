## 1. Fallback chain, test-first

- [ ] 1.1 Failing test: with two fake backends where the first times out, the translation is served by the second and the Segment is marked Degraded. Acceptance: the first backend is not consulted again for the following Segment.
- [ ] 1.2 Implement backend selection and failover until 1.1 passes.
- [ ] 1.3 Failing test: with the first backend healthy, translations are served by it and no Segment is marked Degraded. Acceptance: the second backend receives no requests.
- [ ] 1.4 Failing test: a backend that rejects a request as a configuration error does not cause failover; the row carries an error and the backend stays selected. Acceptance: the next Segment still goes to the same backend.
- [ ] 1.5 Implement the failure classification until 1.4 passes.
- [ ] 1.6 Failing test: while degraded, a higher-priority backend that answers a probe is promoted, and subsequent Segments are served by it and not marked Degraded. Acceptance: promotion happens without restarting the Recording.
- [ ] 1.7 Implement re-probing and promotion until 1.6 passes.
- [ ] 1.8 Failing test: with every backend failing, Segments enter the transcript with original text and no translation, and transcription is unaffected. Acceptance: no Segment is lost from the original panel.

## 2. Persistence

- [ ] 2.1 Carry the Degraded flag on the persisted translation record. Acceptance: reopening a Recording shows the same lines marked as during the meeting.

## 3. Configuration

- [ ] 3.1 Replace the single backend entry with an ordered list, plus a per-request timeout and a re-probe interval. Acceptance: startup validation rejects an empty list naming the key.
- [ ] 3.2 Populate the default list: the LAN backend first, the local backend second. Acceptance: the local entry is verified reachable before this task is marked done.

## 4. Panel marking, tests after

- [ ] 4.1 Mark Degraded rows visibly in the translation panel. Acceptance: the marking is legible without hovering and does not shift row alignment.
- [ ] 4.2 Show an error state on rows whose translation was rejected outright. Acceptance: distinguishable from both pending and Degraded.

## 5. Verification

- [ ] 5.1 Smoke run with the LAN backend unreachable. Acceptance: every line is translated by the local backend and every row is marked Degraded.
- [ ] 5.2 Bring the LAN backend up mid-Recording. Acceptance: within one re-probe interval, new rows are served by it and stop being marked.
- [ ] 5.3 Point the first backend at a nonexistent model. Acceptance: rows show an error, the backend is not demoted, and the mistake is obvious from the screen.
