## Context

The whisper server already answers every `verbose_json` request with three language fields: `language` (the one it decoded with), `detected_language` (what it heard), and `language_probabilities` (a code-keyed distribution). The names are full English words — `"english"`, `"russian"` — while `target_language` in configuration and `language_probabilities` keys are ISO 639-1 codes. Any comparison between the two sides needs one canonical form.

Sending no `language` field is not the same as asking for detection: whisper's own default is `en`, so an omitted field means "decode as English" on some builds. Detection has to be requested explicitly.

## Goals / Non-Goals

**Goals:**

- A Recording gets the right source language without the operator configuring one.
- A line already in the target language reaches the screen without a backend round trip.
- While something is in flight, the window says what and how much.

**Non-Goals:**

- Language identification per speaker or per phrase inside a Stream. Detection stays per Segment, which is what whisper gives us for free.
- Re-translating a Segment whose language was detected wrongly. A wrong detection costs that one line.
- A translated-into-N-languages transcript. The target language stays single and configured.

## Decisions

**Codes are the canonical form; names are an input format.** A small table maps whisper's language names to ISO 639-1 codes both ways, and everything downstream compares codes. An unrecognised language canonicalises to nothing rather than to itself, and an unknown source language never counts as matching the target — so the failure mode of an unknown name is a wasted translation, not a silently untranslated line.

**A configured language overrides detection, including the recorded one.** When a Stream has a language configured, the request forces it, so what whisper reports as detected is irrelevant: the text really is decoded as the configured language. The Segment records the configured language, not the detection.

**Same-language is derived, not persisted.** The Segment records its source language; whether that equals the target is computed when rows are built, from the Recording's own `target_language` in its metadata. Persisting a translation record holding a copy of the original text would double the text on disk and lie about a request that never happened.

**Detection is requested with `language=auto`.** Explicit, and it survives a server whose default is `en`.

**In-flight rows live only in the live view.** A row appears when a Segment closes, carrying its Speaker tag and timestamp with no text yet. It is never written to the transcript file — the store still only sees Segments that transcribed and passed the confidence floor. Its identifier is allocated when the row appears, so the row is filled in place when the text lands rather than being removed and re-inserted.

**A failed transcription keeps its row.** Before this change a whisper failure left nothing on screen; the only trace was a line on stderr. The row stays, marked failed, and is still never persisted.

**The status line is derived from the live view.** In-flight counts, the most recent detected language per Stream, and whether the last translation was Degraded are all readable from the snapshot the panels already receive, so no second status channel is introduced.

## Risks / Trade-offs

- **Detection flaps on short Segments.** A two-word Segment can detect as the wrong language, and with no locking the next Segment may disagree. The status line shows the current detection per Stream, so flapping is visible; locking would trade that for a first-Segment mistake that poisons the whole Recording.
- **A wrong detection can skip a translation.** If a Russian line is detected as the target language, it reaches the translation column untranslated but marked as same-language, which reads as a deliberate state rather than an error.
- **Live view now holds rows that are not Segments.** In-flight and failed rows exist only on screen. The invariant that keeps this honest is that only transcribed rows above the floor are ever appended to the store.
