## 1. Language codes, test-first

- [x] 1.1 Failing test: whisper's language names and ISO 639-1 codes canonicalise to the same code, and an unrecognised language canonicalises to nothing. Acceptance: comparing an unknown language against the target language is false, not true.
- [x] 1.2 Implement the canonical form until 1.1 passes.

## 2. Detection through the transcriber

- [x] 2.1 Failing test: a Stream with no configured language sends a request asking the server to detect it; a configured one sends that language. Acceptance: the detection request is explicit, not an omitted field.
- [x] 2.2 Failing test: a verbose_json reply's language fields parse into a canonical code, whether the reply carries a code-keyed distribution or only a language name. Acceptance: a reply carrying neither yields no language rather than a wrong one.
- [x] 2.3 Implement the transcriber changes until 2.1 and 2.2 pass.

## 3. Source language on the Segment

- [x] 3.1 Failing test: a transcribed Segment carries the detected language, and a Stream with a configured language carries the configured one instead. Acceptance: the configured language wins even when the server reports detecting another.
- [x] 3.2 Failing test: a transcript file whose Segment records carry no source language reads back with none, unchanged otherwise.
- [x] 3.3 Implement until 3.1 and 3.2 pass.

## 4. Same-language bypass

- [x] 4.1 Failing test: a Segment whose source language is the target language reaches the transcript with no translate backend contacted, and its row shows its own text marked as same-language. Acceptance: no translation record is written for it.
- [x] 4.2 Failing test: a Segment whose source language is unknown is still translated.
- [x] 4.3 Failing test: reopening a Recording shows its same-language Segments the same way, derived from the Recording's own target language.
- [x] 4.4 Implement until 4.1 through 4.3 pass.

## 5. In-flight rows

- [x] 5.1 Failing test: a row appears when a Segment closes, before its transcription returns, carrying Speaker tag and timestamp and marked as in flight. Acceptance: it is not written to the transcript file.
- [x] 5.2 Failing test: the row is filled in place when the text arrives, removed when the Segment falls below the confidence floor, and marked failed when the request fails.
- [x] 5.3 Implement until 5.1 and 5.2 pass.

## 6. Status surface

- [x] 6.1 Failing test: the status derived from a live-view snapshot reports in-flight transcription and translation counts, the latest source language per Stream, and whether the last translation was Degraded.
- [x] 6.2 Implement until 6.1 passes.
- [x] 6.3 Status line in the window carrying that status while a Recording is active. Acceptance: it reports nothing in flight when idle.
- [x] 6.4 Running indicator on rows awaiting transcription or translation, stopping when the row settles.
- [x] 6.5 Same-language and transcription-failure marks on rows, distinct from pending, Degraded and translation-error marks.

## 7. Configuration and verification

- [x] 7.1 Both per-Stream language keys absent starts normally with detection on both Streams. Acceptance: the example configuration documents them as optional overrides.
- [x] 7.2 Record a meeting with one Stream in the target language. Acceptance: those rows appear marked same-language with no backend request, the other Stream translates as before, and the status line reports both languages.
