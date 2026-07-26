## Why

The machine already runs a whisper server and a PipeWire graph carrying both the meeting audio and the microphone, but nothing joins them, so a meeting is gone the moment it ends. This change builds the spine end to end — capture both sides, cut speech into Segments, transcribe, order, persist, show — because every risky assumption in the product lives on that path and none of it is proven until a spoken sentence reaches the screen.

Everything else in the PRD hangs off this slice. Until it works there is nothing to translate, nothing to list in history, and nothing for a wizard to configure.

## What Changes

- A Recording starts with its Mic Stream and Monitor Stream taken from TOML configuration. Selecting them interactively arrives in a later change.
- Both Streams are captured concurrently as 16 kHz mono PCM and written to their own WAV file for the life of the Recording.
- Voice activity detection cuts each Stream into Segments independently: a Segment opens on speech, closes after a silence hold, and is force-closed at a duration cap so an uninterrupted monologue still reaches the screen.
- Each Segment is transcribed by the whisper server. Segments whose transcription confidence falls below a configured floor are discarded rather than shown, because whisper produces confident-looking text for non-speech input.
- Segments from both Streams are tagged `me` or `them` and ordered by start timestamp into one transcript, regardless of the order transcriptions come back in.
- Each Segment is appended to the Recording's `segments.jsonl` as it finalizes, so a crash costs the last line rather than the meeting.
- A single window shows the original-transcription panel, filling as people speak, plus a control to stop the Recording.

## Capabilities

### New Capabilities

- `meeting-capture`: capturing two Streams, cutting and transcribing Segments, ordering them into a transcript, and persisting a Recording to disk
- `transcript-view`: the application window and its original-transcription panel
- `app-config`: loading and validating the TOML configuration file

### Modified Capabilities

None. This is the first change.

## Impact

New Rust binary with a Slint user interface. Depends on the `pw-record` binary from PipeWire being present, a reachable whisper server at the configured URL, and the `earshot` crate for voice activity detection. All traffic is to localhost. Introduces the on-disk Recording directory layout that every later change reads.
