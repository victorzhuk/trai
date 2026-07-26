# Meeting transcribe and translate — PRD

## Problem statement

Meetings happen in a language you follow but don't want to reconstruct from memory afterwards, and the participants who matter are on the other end of a call. Existing options are a cloud service that wants the audio of a work meeting, or nothing. What's missing is a local record: what was said, in the original, next to what it means, written to disk as the meeting runs — including your own half of the conversation, which is where the commitments you made live.

The pieces are already on this machine — a whisper server, a PipeWire graph carrying both the meeting audio and the microphone, an LLM box on the LAN — but nothing joins them, so today the meeting is gone the moment it ends.

## Solution

A single-window Linux desktop app in Rust and Slint. You start a [[Recording]] from a small wizard, the app captures both sides of the meeting as two [[Stream]]s, and two side-by-side panels fill as people talk: the left panel with the original transcription, the right with the translation into your configured language. Every line carries a [[Speaker tag]] so `me` and `them` stay distinguishable.

Everything is written to disk while the meeting runs, so a crash costs the last sentence rather than the meeting. Afterwards, history lists past Recordings, reopens any of them in the same two panels, and deletes the ones you don't need. Configuration is a TOML file — there is no settings UI.

Transcription goes to the whisper server already running on this machine. Translation goes to whichever [[Translate backend]] answers first from an ordered list, so a meeting doesn't lose its right-hand panel when the fast machine is off.

## User stories

1. As a meeting participant, I want to start a Recording from a small wizard, so that I can begin capturing within a few seconds of the meeting starting.
2. As a meeting participant, I want the wizard to preselect my usual microphone and monitor sources, so that the common case is one click.
3. As a meeting participant who changes headsets, I want the wizard to list the PipeWire sources currently present, so that I can pick the one I actually plugged in without editing config.
4. As a meeting participant, I want a title prefilled from the current timestamp and editable, so that naming is optional but possible.
5. As a listener, I want the original transcription to appear in the left panel as people speak, so that I can follow and re-read what was just said.
6. As a listener, I want each line's translation to appear in the right panel at the same row, so that I can read one against the other.
7. As a listener, I want a placeholder in the right panel the moment a line appears on the left, so that I can see a translation is coming rather than wonder whether the line was dropped.
8. As a listener, I want every line tagged `me` or `them`, so that the transcript reads as a conversation rather than a wall of text.
9. As a listener, I want lines from both Streams ordered by when they were spoken, so that the transcript matches the meeting even though the two Streams are transcribed independently.
10. As a listener, I want lines translated by a fallback backend visibly marked [[Degraded]], so that a drop in translation quality is something I notice rather than something I discover later.
11. As a meeting participant, I want the transcript written to disk as it is produced, so that a crash or a kill mid-meeting costs at most the last line.
12. As a meeting participant, I want both Streams saved as WAV files, so that I can re-listen or re-transcribe later with a better model.
13. As a meeting participant, I want to stop a Recording and have it finalized and appear in history, so that the meeting is filed without extra steps.
14. As a returning user, I want a history list showing title, date, duration and line count, so that I can find the meeting I'm looking for.
15. As a returning user, I want to reopen a past Recording in the same two panels, so that reviewing a meeting uses the interface I already know.
16. As a returning user, I want to delete a Recording behind a confirmation, so that I can clear meetings I don't need without leaving stray audio on disk.
17. As the operator of this machine, I want configuration in a TOML file, so that I can change target language, backends and defaults in an editor and under version control.
18. As the operator, I want an ordered list of Translate backends in config, so that switching from the LAN box to a local model is an edit rather than a rebuild.
19. As a privacy-conscious user, I want transcription and the fallback translation to run locally, so that a work meeting does not leave my machines by default.

## Implementation decisions

**Capture.** Two `pw-record` subprocesses per Recording, one bound to the microphone source, one to the output sink's `.monitor`, each invoked with `--target <node>`, `--rate 16000`, `--channels 1`, `--format s16`, `--raw`, writing raw PCM to stdout. The app reads each stdout, tees the samples to that Stream's WAV writer and to its segmenter. Source enumeration for the wizard comes from the PipeWire source list, preselected to the names in config.

**Segmentation.** Voice activity detection per Stream using `earshot` (pure Rust, no C or ONNX dependency). A [[Segment]] opens on detected speech, closes after a silence hold of roughly 700ms, and is force-closed at a hard cap of roughly 20s so an uninterrupted monologue still reaches the panels. Both thresholds are config values, since they are the parameters most likely to need tuning against a real meeting. The Segment carries its start and end timestamps relative to Recording start; these timestamps, never response arrival order, determine its position in the transcript.

**Transcription.** One `POST /inference` per Segment to the existing whisper server, multipart with `file`, `response_format=verbose_json`, `temperature=0.0`, and `language` when a source language is pinned in config. Requests are fired as Segments close, with no application-side queue: the server serializes on its single context, and measured throughput is roughly 7.8x realtime aggregate against a two-Stream demand of 2x realtime. Both the serialization and the headroom are recorded in ADR-0001.

**Hallucination filtering.** Whisper emits confident-looking text for non-speech input — a 10s sine tone produced `" Продолжение следует..."` during verification. VAD gating removes most of the exposure by never submitting silence. As a second filter, a Segment whose mean word probability from `verbose_json` falls below a configured floor is discarded rather than shown. The floor is a config value.

**Translation.** Any OpenAI-compatible `POST /v1/chat/completions` endpoint. Config holds an ordered list of backends, each with `base_url`, `model` and optional `api_key`. The first reachable backend wins; on connection failure or timeout the pipeline drops to the next and marks resulting Segments [[Degraded]], periodically re-probing to climb back. The default first entry is the LM Studio instance on the LAN GPU box; the local Ollama endpoint on this machine is the fallback. Target language is a single config value applied to every Recording — whisper's own translate task is deliberately not used, since it can only produce English.

**Translate prompt.** One request per Segment, carrying that Segment's text plus the previous three translated lines as rolling context, so that pronouns, names and recurring jargon stay consistent across a conversation. The response is the translation of the current Segment only.

**Panels.** When a Segment closes, both panels receive a row at the same index — the left with the transcription once it returns, the right with a pending placeholder replaced in place when translation completes. Row position derives from the Segment's start timestamp, which absorbs out-of-order replies from either the whisper server or the translate backend. Scroll is locked across the two panels so a line can be read against its translation.

**Storage.** One directory per Recording under the [[Store root]], holding `meta.json` (title, start time, duration, source names, target language, backend used), `segments.jsonl` appended line by line as Segments finalize, and `mic.wav` plus `monitor.wav`. Appending JSONL live is what makes a mid-meeting crash survivable. History lists the Store root's subdirectories; deleting a Recording removes its directory. No database and no schema migration.

**History view.** Reopening a Recording replays `segments.jsonl` into the same two panels with input disabled, reusing the live view rather than introducing a second transcript renderer.

**Configuration.** TOML only, no settings UI: Store root, default microphone and monitor source names, target language, optional per-Stream source language, VAD thresholds, probability floor, and the ordered Translate backend list.

## Testing decisions

- **Flow:** test-first for the pure core, tests-after for the shell. The VAD gate, the timestamp interleave and the backend fallback chain are written red-green, because a wrong flush rule or a mis-ordered transcript is expensive to diagnose by listening to a recording. Capture wiring, Slint views and history get tests after they work, or a smoke run against the live whisper server.
- **Seam:** the pipeline core takes `Transcriber` and `Translator` as traits. Tests drive it with fakes — a transcriber that returns canned text after a controllable delay, a translator that can be made to fail or time out. No network, no PipeWire, no UI in the test path.
- **What the tests cover.** Fallback: first backend times out, second answers, resulting Segments are marked [[Degraded]], and the pipeline climbs back to the first once it recovers. Ordering: replies delivered out of order still produce a transcript sorted by Segment start timestamp, with `me` and `them` correctly interleaved. VAD gating: a synthetic sample stream opens a Segment on speech onset, closes it after the silence hold, and force-closes at the duration cap. Filtering: a Segment below the probability floor is discarded. Store: append-then-read round-trips `segments.jsonl`, and a truncated final line — the crash case — is tolerated on read rather than failing the whole Recording.
- **Prior art:** none. The repository is greenfield; these are the first tests and the first seam.

## Out of scope

- Speaker diarization. Attribution is by capture source only — every remote participant arrives as `them`.
- Summarization, action-item extraction, or any post-meeting LLM pass over the transcript.
- Live editing or correction of transcribed lines.
- Search across Recordings.
- Export to any format. The transcript is readable in-app and as JSONL on disk.
- A settings UI, a preferences dialog, or any in-app configuration editing.
- Per-Recording target language, and per-Recording save-path selection.
- Packaging, installers, and autostart integration.
- Non-Linux platforms, and audio backends other than PipeWire.
- Retrying or backfilling translations for a Recording after it has been stopped.

## Further notes

Open questions carried over from the grill session, unresolved and not invented here:

1. **LM Studio endpoint.** The `host:port` of the LAN box is unknown, and it could not be probed from this machine — `ip neigh` shows only the Docker bridge. If LM Studio runs inside WSL rather than natively on Windows, it sits behind WSL's NAT and needs a `netsh interface portproxy` rule before any LAN peer can reach it. Until this is verified, the primary Translate backend is unconfirmed and every meeting falls back to the local model.
2. **Target language.** Never stated. The whole right-hand panel depends on it.
3. **Store root default.** Not chosen.
4. **Per-Stream source language.** Whether to pin the Mic Stream to a known language for accuracy, or leave both Streams on whisper's auto-detect. Auto-detect on short Segments is the weaker case for detection, so this may matter more than it appears.
5. **Local fallback model.** `qwen3.5:4b` is the only non-cloud chat model currently pulled on this machine. Its translation quality for the target pair is untested.
6. **Application and binary name.** The working directory is `trai`; whether that is the name is unconfirmed.

Decisions not reached during grilling, flagged rather than assumed:

7. **Stop semantics.** Whether stopping waits for in-flight Segments to finish transcribing and translating before finalizing, or cuts immediately and leaves the tail untranslated.
8. **Source disappearing mid-Recording.** Behaviour when a headset is unplugged and its `pw-record` process exits — restart against the new default source, or fail that Stream and continue with the other.
9. **Autoscroll.** Whether the panels follow new lines automatically, and whether scrolling up pins the view.

Verified during grilling, recorded here so it is not re-litigated: whisper transcribes 10s of audio in roughly 1.3-1.5s on this CPU; four concurrent requests complete in 5.15s total, confirming server-side serialization with ample headroom; the server honours a forced `language` parameter; `pw-record` supports the raw-stdout invocation the capture design depends on.
