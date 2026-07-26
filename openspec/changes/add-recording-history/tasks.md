## 0. Blocking decision

- [ ] 0.1 Decide stop semantics: whether stopping waits for in-flight Segments or cuts immediately. Acceptance: the answer is recorded before task 2.1 is implemented, since it determines what duration and line count mean.

## 1. History reading, test-first

- [ ] 1.1 Failing test: listing a Store root containing several Recording directories returns them newest first with title, date, duration and line count. Acceptance: a directory missing its metadata file is skipped rather than failing the whole listing.
- [ ] 1.2 Implement history listing until 1.1 passes.
- [ ] 1.3 Failing test: reading a Recording's transcript merges Segment and translation records into ordered lines carrying original text, translation, Speaker tag and Degraded mark. Acceptance: a Segment with no translation record still appears.
- [ ] 1.4 Implement transcript reading until 1.3 passes, reusing the merge from the translation change rather than duplicating it.

## 2. Finalization

- [ ] 2.1 Write the Recording's metadata on stop: title, start time, duration, captured source names, target language. Acceptance: the Recording appears in history immediately after stopping.

## 3. History view, tests after

- [ ] 3.1 List view showing title, date, duration and line count, newest first. Acceptance: a Recording stopped moments ago is at the top.
- [ ] 3.2 Opening a Recording replays its transcript into the two panels with input disabled. Acceptance: Degraded marks appear as they did live.
- [ ] 3.3 Returning from a reopened Recording to the list. Acceptance: no Recording is started by navigating.
- [ ] 3.4 Delete behind a confirmation naming the Recording. Acceptance: confirming removes the directory including audio; dismissing changes nothing on disk.

## 4. Verification

- [ ] 4.1 Record, stop, and reopen. Acceptance: the reopened transcript matches what was on screen during the Recording, line for line.
- [ ] 4.2 Delete a Recording and confirm the directory is gone. Acceptance: the list refreshes without it and no orphaned audio remains in the Store root.
