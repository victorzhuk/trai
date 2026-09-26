# Context

Glossary for the meeting transcribe/translate app. Terms only — no implementation detail.

## Recording

One meeting capture from start to stop. Owns two [[Stream]]s, one ordered list of [[Segment]]s, the saved audio files, and the metadata shown in history (title, start time, duration, languages).

*Avoid:* call, session, meeting (the meeting is the real-world event; the Recording is our artifact of it).

## Stream

One capture input inside a Recording. Exactly two per Recording:

- **Mic Stream** — the local microphone, a PipeWire source of kind *input*. Carries [[Speaker tag]] `me`.
- **Monitor Stream** — the output sink's `.monitor`, i.e. what the meeting app plays. Carries [[Speaker tag]] `them`.

The OS-level node a Stream binds to is a *PipeWire source*; the Stream is our per-Recording capture of it.

*Avoid:* channel (means stereo L/R here), track, input.

## Speaker tag

Which side of the meeting a [[Segment]] came from: `me` (Mic Stream) or `them` (Monitor Stream). Not speaker diarization — no identification of individuals inside the Monitor Stream.

*Avoid:* speaker, participant, diarization.

## Segment

One unit of speech, cut from a single [[Stream]], sent to whisper as one request. Has: stream, speaker tag, start/end timestamp relative to Recording start, original text, translated text. The two Streams' Segments interleave by start timestamp to form the Recording's transcript.

A Segment ends when speech stops, not on a clock — voice activity detection closes it after a silence hold, with a hard duration cap so one unbroken monologue still reaches the panels.

*Avoid:* chunk (reserved for the raw audio buffer before it becomes a Segment), utterance, phrase.

## Transcribe backend

The transcription endpoint a Recording's [[Segment]]s are sent to. Exactly one per run, set by config: either a local whisper.cpp server or an OpenAI-compatible cloud endpoint. Cloud is the operator's explicit opt-in — with it, meeting audio leaves the machine.

*Avoid:* whisper server (that's the local instance, not the concept), STT.

## Translate backend

An OpenAI-compatible chat endpoint that turns a [[Segment]]'s original text into the configured target language. Config lists them in priority order; the first reachable one wins.

*Avoid:* translator, LLM, provider.

## Degraded

State of a [[Segment]] translated by any [[Translate backend]] other than the first in the list, i.e. after a fallback. Marked in the translation panel so a quality drop is visible rather than silent.

*Avoid:* fallback (that's the act), error, stale.

## Store root

The directory holding every [[Recording]]'s own subdirectory. One Recording = one subdirectory = the unit of deletion in history.

*Avoid:* library, archive, database.
