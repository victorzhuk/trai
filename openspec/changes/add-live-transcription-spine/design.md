## Context

Greenfield repository. The whisper server is an existing, separately managed process reached over HTTP; the audio graph is PipeWire. Two facts were measured rather than assumed during grilling: the whisper server serializes concurrent requests on a single context, and it transcribes 10s of audio in roughly 1.3-1.5s on this CPU, giving roughly 7.8x realtime aggregate throughput against this design's 2x realtime demand. ADR-0001 records why capture is dual-stream despite doubling that demand.

## Goals / Non-Goals

**Goals:**

- A spoken sentence reaches the screen, correctly attributed and correctly ordered, without a queue or a scheduler.
- The transcript on disk is complete up to the last finalized Segment at any instant, including after a kill.
- The pure logic — segmentation, ordering, filtering, storage — is testable without PipeWire, without the network, and without a user interface.

**Non-Goals:**

- Translation, and the second panel. The next change owns those.
- Interactive source selection, history, and deletion.
- Recovering audio for a Segment discarded by the confidence filter.

## Decisions

**Capture via subprocess, not a PipeWire client library.** Each Stream is a `pw-record` process emitting raw PCM to stdout, which the application reads. Binding libpipewire directly would remove a process boundary and give finer control over stream lifecycle, but costs a native dependency and substantially more code for a slice that has to work tomorrow. The subprocess boundary also isolates a crashing capture from the rest of the application.

**No application-side transcription queue.** Segments are submitted as they close. The whisper server serializes them itself, and the measured headroom is roughly 4x what two live Streams demand. A queue would add state to reason about in exchange for solving a problem the measurement says does not exist. If a heavier model is swapped in later, this is the first assumption to recheck.

**Timestamps are authoritative for ordering, not arrival.** A Segment's position in the transcript comes from its start timestamp relative to Recording start. The two Streams transcribe independently and replies return out of order; ordering by arrival would silently scramble a fast exchange. This also means a late reply inserts into the middle of the transcript rather than appending, which the panel's row model must accommodate.

**Append-only JSONL, no database.** Each finalized Segment is one line appended and flushed. A crash truncates at most the final line, and a reader that tolerates a truncated final line loses nothing else. A database would buy queries this product does not have, at the cost of a schema and migrations for a store holding tens of records.

**Confidence floor over prompt tricks.** Verification showed the server returning `" Продолжение следует..."` for a 10s sine tone. Voice activity detection removes most non-speech before it is ever submitted; the mean word probability from `verbose_json` catches the rest. The floor is configurable because the right value is empirical.

**Trait seam at `Transcriber`.** The pipeline core depends on a trait, not on the HTTP client. Tests drive it with a fake that returns canned text after a controllable delay, which is what makes out-of-order ordering testable at all.

## Risks / Trade-offs

- **Voice activity thresholds are guesses until a real meeting runs.** A silence hold too short fragments sentences and hurts transcription accuracy; too long delays the panel. Both are configuration values specifically so tuning does not require a rebuild.
- **A Segment force-closed at the duration cap cuts mid-sentence.** Accepted: the alternative is an unbounded Segment that never reaches the screen during a monologue.
- **`pw-record` exiting mid-Recording is unhandled here.** The PRD flags the behaviour as an open question; this change does not invent an answer, so an unplugged headset currently ends that Stream silently.
- **The confidence floor can discard real speech.** Quiet or heavily accented speech may fall below it. Mitigated by making the floor configurable and defaulting it low enough to catch only the obvious hallucination case.
