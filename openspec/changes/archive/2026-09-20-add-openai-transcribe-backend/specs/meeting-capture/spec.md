## MODIFIED Requirements

### Requirement: Segment transcription

Each closed Segment SHALL be submitted to the configured Transcribe backend as a single request, in the dialect selected by configuration. The request SHALL ask for a response format that carries per-segment confidence. When a source language is configured for a Stream, that language SHALL be sent with the request in whichever form the dialect accepts; when none is configured, the request SHALL let the server detect the language in the dialect's own way. Requests SHALL be submitted as Segments close, without an application-side queue.

#### Scenario: Segment closes

- **WHEN** a Segment closes on either Stream
- **THEN** a transcription request for that Segment is submitted immediately

#### Scenario: Source language configured

- **WHEN** a Stream has a source language configured
- **THEN** the transcription request for its Segments carries that language rather than relying on auto-detection

#### Scenario: No source language configured

- **WHEN** a Stream has no source language configured
- **THEN** the transcription request asks the server to detect the language rather than defaulting to one — carrying `language=auto` in the `whisper.cpp` dialect, omitting the field in the `openai` dialect, which rejects `auto`

### Requirement: Low-confidence Segment discard

A transcribed Segment whose confidence falls below the configured floor SHALL be discarded: it SHALL NOT appear in the transcript and SHALL NOT be persisted. Confidence SHALL be derived from per-word probabilities when the response provides them; otherwise from the exponential of the mean per-segment log-probability — the geometric-mean word probability — so the same `confidence_floor` value governs both dialects.

#### Scenario: Non-speech audio transcribed into text

- **WHEN** a Segment transcribes to text whose confidence is below the configured floor
- **THEN** the Segment is discarded and never appears in the transcript

#### Scenario: Confident transcription

- **WHEN** a Segment transcribes with confidence at or above the floor, whether from mean word probability or from `exp(mean avg_logprob)`
- **THEN** the Segment enters the transcript unchanged

#### Scenario: Hallucinated transcription with segment log-probabilities only

- **WHEN** a Segment's response carries no per-word probabilities and `exp(mean avg_logprob)` is below the floor
- **THEN** the Segment is discarded, exactly as a low word-confidence Segment would be
