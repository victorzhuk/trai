# Design — add-recording-history

## Stop semantics (task 0.1)

**Decision: wait for in-flight Segments.** `Recording::stop()` already joins
both reader threads, joins every outstanding submission thread, and calls
`pipeline.drain_translations()` before returning. Audio capture is cut when
`pw-record` is stopped, but PCM already buffered in the readers is still
segmented, submitted, transcribed, and translated before stop returns.

Consequences:

- **Duration** = wall-clock span from Recording start to stop (the capture
  window), computed via a monotonic `Instant` captured at start.
- **Line count** = every Segment whose speech began before stop, including
  the tail that finishes transcribing after the user clicks Stop. The
  transcript is complete; Stop blocks briefly for the tail.

No code change is needed to enforce waiting — the current `stop()` already
does it. The decision only fixes what `duration` and `line count` mean in
history and in the finalization metadata.

## Metadata format

`meta.json` is written at start (crash safety, preserves the existing
`meta_json_is_written_at_start` test) and finalized on stop. The
`RecordingMeta` struct gains:

- `start_time: u64` — Unix seconds, written at start (drives date display
  and newest-first ordering).
- `target_language: String` — written at start (known from config).
- `duration_secs: Option<u64>` — written on stop; `None` if the app was
  killed before stopping. History listing treats a missing duration as
  "unknown" rather than failing.

A directory killed before stop still has a readable `meta.json` (written at
start), so it is listable; it simply has no duration. A directory with no
readable `meta.json` is skipped by the listing.

## History reading (tasks 1.1-1.4)

New `src/history.rs` module:

- `list_recordings(store_root) -> io::Result<Vec<RecordingEntry>>` — scans
  the store root subdirectories, reads `meta.json`, counts Segment lines via
  `store::read_all`, sorts newest first by `start_time`. Directories without
  readable metadata are skipped, not fatal.
- `read_transcript(dir) -> io::Result<Vec<Segment>>` — thin wrapper over
  `store::read_all(segments.jsonl)`, reusing the existing Segment/Translation
  merge rather than duplicating it. The UI replays via `build_rows` on the
  result, so Degraded marks and Speaker tags match the live view.

`RecordingEntry { dir, title, start_time, duration_secs, line_count }`.

## Finalization (task 2.1)

`RecordingParams` gains `target_language: String`. `Recording` captures a
`start: Instant` and a `start_time: u64` (wall clock) at start, writes the
extended `meta.json` at start, and on stop rewrites `meta.json` with
`duration_secs = Some(start.elapsed().as_secs())`.

## History view (tasks 3.1-3.4)

Slint (`ui/main_window.slint`) gains a history list shown when no recording
is active and the wizard is closed, a replay view that loads a past
Recording's transcript into the existing two-panel layout with input
disabled, and a delete confirmation naming the Recording. New callbacks:
`history-clicked`, `replay-back-clicked`, `delete-clicked`,
`confirm-delete-clicked`, `cancel-delete-clicked`. `main.rs` wires them to
`history::list_recordings`, `history::read_transcript` + `build_rows`, and
`fs::remove_dir_all`.

## Testing mode

Tasks name the approach: TDD (failing test first) for 1.1/1.3,
implement-to-pass for 1.2/1.4, tests-after for UI (3.x), integration
verification for 4.1/4.2. UI rendering is covered by reusing the already
tested `build_rows`; the new UI wiring is exercised through the history
module's on-disk contracts (list/read/delete) in `tests/`.

## Verification floor

`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
