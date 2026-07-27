# translation Specification

## Purpose
TBD - created by archiving change add-translation-panel. Update Purpose after archive.
## Requirements
### Requirement: Segment translation into the configured target language

Every Segment that enters the transcript SHALL be translated into the single target language taken from configuration, by sending one request per Segment to an OpenAI-compatible chat endpoint. The target language SHALL NOT be selectable per Recording.

#### Scenario: Segment enters the transcript

- **WHEN** a Segment is transcribed and enters the transcript
- **THEN** exactly one translation request is made for that Segment, into the configured target language

#### Scenario: Segment discarded by the confidence filter

- **WHEN** a Segment is discarded before entering the transcript
- **THEN** no translation request is made for it

### Requirement: Rolling translation context

A translation request SHALL carry the Segment's own text together with the previous three translated lines of the same Recording as context, and no more.

#### Scenario: Fifth line of a Recording is translated

- **WHEN** the fifth Segment of a Recording is translated
- **THEN** the request carries the three preceding translated lines and does not carry the first line

#### Scenario: First line of a Recording is translated

- **WHEN** the first Segment of a Recording is translated
- **THEN** the request carries no context lines and still produces a translation

### Requirement: Translation persistence as append-only records

A completed translation SHALL be appended to the Recording's transcript file as its own record referring to its Segment's identifier. The transcript file SHALL remain append-only. Reading a transcript SHALL merge Segment records with their translation records.

#### Scenario: Translation returns after its Segment was persisted

- **WHEN** a translation completes for a Segment already written to the transcript file
- **THEN** a translation record referring to that Segment is appended, and no existing line is rewritten

#### Scenario: Recording ends with a translation outstanding

- **WHEN** a transcript is read in which one Segment has no translation record
- **THEN** that Segment is returned with its original text and no translation, and the remaining Segments are unaffected

