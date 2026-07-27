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

### Requirement: Ordered translate backend list

Translation SHALL be served by the first reachable backend in the configured ordered list. The list SHALL be a priority order, not a pool: requests SHALL NOT be distributed across backends.

#### Scenario: First backend is healthy

- **WHEN** Segments are translated and the first configured backend answers
- **THEN** every request goes to the first backend and no other backend is contacted

#### Scenario: First backend is unreachable at the start of a Recording

- **WHEN** a Recording begins while the first backend refuses connections
- **THEN** translation is served by the next backend in the list

### Requirement: Failover on unavailability

A connection failure, a timeout, or a server-side error against the serving backend SHALL move translation to the next backend in the list. A rejection indicating misconfiguration, such as an unknown model or a refused key, SHALL NOT cause failover.

#### Scenario: Serving backend times out

- **WHEN** a translation request exceeds the configured timeout
- **THEN** translation moves to the next backend and subsequent Segments are served by it

#### Scenario: Serving backend rejects the request as misconfigured

- **WHEN** a backend rejects a request because the configured model is unknown
- **THEN** the affected row reports an error, the backend remains selected, and the next Segment is sent to the same backend

#### Scenario: Every backend fails

- **WHEN** no configured backend can serve a translation
- **THEN** the Segment enters the transcript with its original text and no translation, and transcription is unaffected

### Requirement: Degraded marking

A Segment translated by any backend other than the first in the list SHALL be marked Degraded. The mark SHALL be persisted with the translation so that it survives reopening the Recording.

#### Scenario: Translation served by a fallback backend

- **WHEN** a Segment is translated by a backend other than the first
- **THEN** that Segment is marked Degraded on screen and in the transcript file

#### Scenario: Recording reopened later

- **WHEN** a Recording containing Degraded Segments is reopened
- **THEN** the same Segments are still shown as Degraded

### Requirement: Recovery re-probing

While translation is served by a fallback, the application SHALL periodically probe higher-priority backends with a lightweight request at the configured interval, and SHALL return to the first one that answers. Probing SHALL NOT consume or duplicate a Segment's translation.

#### Scenario: Preferred backend becomes available mid-Recording

- **WHEN** a higher-priority backend answers a probe while a fallback is serving
- **THEN** subsequent Segments are translated by the higher-priority backend and are not marked Degraded

#### Scenario: Probe fails

- **WHEN** a probe to a higher-priority backend fails
- **THEN** the current backend continues serving and no Segment is affected by the probe

