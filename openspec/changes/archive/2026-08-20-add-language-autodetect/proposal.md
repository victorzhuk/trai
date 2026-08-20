## Why

Both source languages are pinned in configuration today, so a meeting that switches languages, or one where the far side is not the language the operator guessed, transcribes into the wrong language for its whole duration. Whisper already reports what it detected on every response; nothing reads it.

Pinning also hides the case where a Stream is already in the target language: every one of those Segments is sent to a translate backend to be turned into itself, burning a request and a wait on a line the user can already read.

While a Segment is in flight nothing is on screen either. A row appears only once whisper answers, so during a slow request the panel looks idle, and the translation column's placeholder cannot be told apart from a stalled one.

## What Changes

- A Stream with no configured source language asks whisper to detect it, and the detected language is recorded per Segment. Configured languages stay as explicit overrides.
- A Segment whose source language matches the target language is not sent to a translate backend: the translation column shows its text as it stands, marked as being in the same language.
- A row appears the moment a Segment closes, showing a spinner in the original column until its transcription returns, and a failed transcription leaves a visible row rather than nothing.
- The window carries a status line while a Recording is active: detected language per Stream, how many Segments are transcribing and translating, and whether translation is running on a fallback backend.

## Capabilities

### Modified Capabilities

- `meeting-capture`: source language detection per Segment
- `translation`: same-language Segments bypass the backends
- `transcript-view`: in-flight rows, spinners, and a status line
- `app-config`: the per-Stream language keys become optional overrides

## Impact

The transcript record gains a source language field; older recordings without it read back unchanged and simply show no detected language. Detection costs nothing extra per request — whisper already returns it.

Detection is per Segment with no locking, so a Stream can flip language between lines and, on short or noisy Segments, can flip wrongly. That is visible in the status line rather than hidden, and a wrong detection costs one line rather than the Recording.
