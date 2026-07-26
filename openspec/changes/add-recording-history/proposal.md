## Why

Recordings accumulate in the Store root with no way to see or remove them from inside the application that made them. Reviewing yesterday's meeting means finding a directory and reading JSON by hand, and clearing space means deleting directories by hand — with the audio files that make a Recording worth keeping sitting right next to the ones you meant to remove.

Stopping a Recording also needs to actually finish it: the metadata history lists is only complete once the Recording ends.

## What Changes

- Stopping a Recording finalizes its metadata — title, start time, duration, the sources captured, and the target language used.
- A history view lists past Recordings, newest first, showing title, date, duration, and line count.
- Opening a Recording from history replays its transcript into the same two panels, read-only, merging Segment records with their translation records and preserving Degraded marks.
- Deleting a Recording removes its whole directory, audio included, behind a confirmation naming what will be deleted.

## Capabilities

### New Capabilities

- `recording-history`: listing, reopening, and deleting past Recordings

### Modified Capabilities

- `meeting-capture`: stopping a Recording finalizes its metadata

## Impact

Reuses the transcript panels rather than adding a second renderer, so it inherits the merge behaviour and Degraded marking from earlier changes. Deletion is destructive and irreversible — it removes captured audio along with the transcript.

An open question from the PRD remains unresolved and must be answered before the finalization task is implemented: whether stopping waits for in-flight Segments to finish transcribing and translating, or cuts immediately and leaves the tail untranslated. This change does not decide it.
