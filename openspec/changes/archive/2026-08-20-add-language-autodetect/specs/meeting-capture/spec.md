## MODIFIED Requirements

### Requirement: Segment transcription

Each closed Segment SHALL be submitted to the configured whisper server as a single request, requesting a response format that carries per-word confidence. When a source language is configured for a Stream, that language SHALL be sent with the request; when none is configured, the request SHALL ask the server to detect the language. Requests SHALL be submitted as Segments close, without an application-side queue.

#### Scenario: Segment closes

- **WHEN** a Segment closes on either Stream
- **THEN** a transcription request for that Segment is submitted immediately

#### Scenario: Source language configured

- **WHEN** a Stream has a source language configured
- **THEN** the transcription request for its Segments carries that language rather than relying on auto-detection

#### Scenario: No source language configured

- **WHEN** a Stream has no source language configured
- **THEN** the transcription request asks the server to detect the language rather than defaulting to one

## ADDED Requirements

### Requirement: Per-Segment source language

Every Segment SHALL carry the language its text is in, persisted with the Segment. For a Stream with a configured source language that SHALL be the configured language; otherwise it SHALL be the language the server reported detecting. Detection SHALL be evaluated per Segment, with no Stream-level lock-in.

#### Scenario: Stream with no configured language

- **WHEN** a Segment on a Stream with no configured language is transcribed
- **THEN** the Segment carries the language the server detected

#### Scenario: Stream with a configured language

- **WHEN** a Segment on a Stream with a configured language is transcribed
- **THEN** the Segment carries the configured language, whatever the server reports detecting

#### Scenario: Speaker switches language mid-Recording

- **WHEN** consecutive Segments on the same Stream are detected as different languages
- **THEN** each Segment carries its own detected language

#### Scenario: Recording made before source languages were recorded

- **WHEN** a transcript whose Segments carry no source language is read
- **THEN** every Segment is returned with no source language and the transcript is otherwise unaffected
