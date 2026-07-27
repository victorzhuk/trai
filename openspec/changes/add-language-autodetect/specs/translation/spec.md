## MODIFIED Requirements

### Requirement: Segment translation into the configured target language

Every Segment that enters the transcript and is not already in the target language SHALL be translated into the single target language taken from configuration, by sending one request per Segment to an OpenAI-compatible chat endpoint. The target language SHALL NOT be selectable per Recording.

#### Scenario: Segment enters the transcript

- **WHEN** a Segment whose source language differs from the target language is transcribed and enters the transcript
- **THEN** exactly one translation request is made for that Segment, into the configured target language

#### Scenario: Segment discarded by the confidence filter

- **WHEN** a Segment is discarded before entering the transcript
- **THEN** no translation request is made for it

## ADDED Requirements

### Requirement: Same-language Segments bypass the backends

A Segment whose source language is the target language SHALL NOT be sent to any translate backend. It SHALL be presented with its own text standing in for the translation, marked as being in the same language, and no translation record SHALL be written for it. A Segment whose source language is unknown SHALL be translated.

#### Scenario: Segment already in the target language

- **WHEN** a Segment is transcribed whose source language is the target language
- **THEN** no translation request is made and the row shows the Segment's own text marked as being in the same language

#### Scenario: Same-language Recording reopened later

- **WHEN** a Recording containing same-language Segments is reopened
- **THEN** those Segments are shown the same way, without their text having been duplicated in the transcript file

#### Scenario: Both languages present in one Recording

- **WHEN** one Stream is in the target language and the other is not
- **THEN** only the Stream that is not in the target language produces translation requests

#### Scenario: Source language could not be determined

- **WHEN** a Segment carries no recognisable source language
- **THEN** it is translated as usual rather than assumed to be in the target language
