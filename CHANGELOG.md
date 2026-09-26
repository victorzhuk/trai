# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-09-26

### Fixed

- **CI pins Rust 1.93.1.** CI runs on the tested toolchain instead of
  floating `stable`. The v0.2.0 tag CI failed on a new Clippy lint
  introduced in Rust 1.98 after the local green build; v0.2.1 carries
  the same source plus the toolchain pin, and is the first tag
  carrying the durable `trai-v0.2.1-linux-x86_64.tar.gz` release asset.

## [0.2.0] - 2026-09-26

### Added

- **CI builds a release binary on every push to `master` or a `v*` tag.**
  The binary is uploaded as a GitHub Actions workflow artifact
  (`trai-linux-x86_64-<commit>`, 14-day retention), so a fresh build
  can be downloaded from the Actions run page. These are transient
  run artifacts, not the durable release distribution: per-tag
  releases attach `trai-vX.Y.Z-linux-x86_64.tar.gz` to the GitHub
  Release only after a green tag CI run and a manual curation step,
  so not every tag automatically produces a release asset.
- **OpenAI-compatible transcription backend.** `whisper_kind = "openai"`
  sends finished segments to an OpenAI-compatible `/v1/audio/transcriptions`
  endpoint, configured with `whisper_api_key` and `whisper_model`. Local
  whisper.cpp remains the default.
- **avg_logprob confidence fallback.** When a transcription response
  carries no per-word probabilities, confidence falls back to the segment's
  `avg_logprob` instead of an unknown value.

### Fixed

- **Failed appends no longer drop a whole batch.** When a segment append
  fails mid-batch, the pipeline restores the unwritten rows to its
  in-memory queue so the next flush retries them in start-ms order; a row
  that landed but could not be fsynced is reported as a typed
  `Durability` error rather than a generic I/O error, so callers do not
  retry into a duplicate write.
- **Recording metadata writes are now atomic.** A temp-file plus
  atomic rename plus directory fsync is used at both start and stop, so
  a mid-recording interruption is more likely to leave a self-describing
  recording on disk rather than a half-written file.
- **Two recordings started in the same second get distinct directories.**
  Recording directory creation uses an exclusive `mkdir` that retries
  through candidates, so concurrent recordings no longer risk
  truncating each other's files.
- **Replay view acts on the open recording.** Starting a new recording
  clears the replay target, so Delete in the history view always hits
  the currently-open replay; segments transcribed but never translated
  after stop render as `—` instead of a perpetually pending spinner.
- **`/models` probe only accepts backends that serve the configured
  model.** A 200 OK from the probe now counts as success only when the
  configured model id appears in the returned list; endpoints that don't
  return a list keep the status-only behavior.
- **Trailing slash in `whisper_url` no longer doubles the path.** The
  base URL is trimmed on client construction, so a configured
  `https://host/` no longer produces a doubled-slash request to
  `/inference` or `/audio/transcriptions`.

## [0.1.1] - 2026-07-27

### Fixed

- Fix two clippy lints that failed CI on Rust 1.97: `manual_filter` in
  `config.rs` (use `Option::filter` instead of `and_then` with a conditional)
  and `unnecessary_sort_by` in `history.rs` (use `sort_by_key` with `Reverse`).

## [0.1.0] - 2026-07-27

First usable release. Dual-stream meeting transcription and translation with
live panels, recording history, language auto-detection, and a force-stop
escape hatch for dead translation backends.

### Added

- **Dual-stream capture.** Records the microphone (`me`) and the output
  monitor (`them`) as two independent PipeWire streams, each with its own
  voice activity detection and transcription pipeline. Segments interleave
  by start timestamp so the transcript reads as a conversation.
- **Live transcription.** Whisper transcription fires as segments close,
  with mid-speech chunking (`live_chunk_ms`, default 3 s) so text appears
  while the speaker is still talking. In-flight rows show a spinner before
  the text lands.
- **Live translation.** Each segment is translated into the configured
  target language via OpenAI-compatible backends. The previous three
  translated lines are sent as rolling context. A pending placeholder
  appears the moment a row opens and is filled in place when the
  translation returns.
- **Backend fallback chain.** Translation backends are configured as an
  ordered list. The first reachable one wins; on failure the pipeline
  falls over to the next and marks resulting segments **degraded**.
  The pipeline periodically re-probes earlier backends to climb back.
- **Language auto-detection.** When no per-stream language is configured,
  whisper's `language=auto` detects the source language per segment.
  Segments already in the target language skip translation and show
  their original text marked **same language**. Configured per-stream
  languages override detection.
- **Recording history.** Past recordings are listed with title, date,
  duration, and line count. Clicking reopens the transcript in the same
  two-panel view. Deletion is behind a confirmation dialog.
- **Source-selection wizard.** A pre-recording wizard enumerates
  PipeWire sources, preselects configured defaults, and lets the user
  pick a different microphone or monitor without editing config.
- **Force stop.** A **Force stop** button appears while a recording is
  stopping. It detaches translation threads so stop returns immediately
  instead of waiting for dead backends to time out.
- **Crash-safe storage.** Segments are appended to `segments.jsonl` as
  they finalize, and both streams are teed to WAV files as they arrive.
  A mid-meeting crash costs at most the last line.
- **Status surface.** The header shows live status chips: detected
  languages per stream, in-flight transcription and translation counts,
  failed transcriptions, and degraded-backend warnings.
- **Configuration.** TOML config file with documented example
  (`config.toml.example`). No settings UI.

[unreleased]: https://github.com/victorzhuk/trai/compare/v0.2.1...HEAD
[0.2.0]: https://github.com/victorzhuk/trai/compare/v0.1.1...v0.2.0
[0.2.1]: https://github.com/victorzhuk/trai/compare/v0.2.0...v0.2.1
[0.1.1]: https://github.com/victorzhuk/trai/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/victorzhuk/trai/releases/tag/v0.1.0
