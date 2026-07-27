# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
