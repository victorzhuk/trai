## ADDED Requirements

### Requirement: Readable text contrast

All text in the window SHALL meet WCAG AA contrast (4.5:1 for normal text) against its background on every supported color scheme. Low-emphasis ink reserved for decoration SHALL NOT be used for text.

#### Scenario: Light scheme status text

- **WHEN** status or caption text is shown on a light scheme
- **THEN** its color passes 4.5:1 contrast against the background

## MODIFIED Requirements

### Requirement: Degraded and failed rows are visibly distinct

Rows whose translation is Degraded and rows whose translation failed SHALL be visually distinct from normal rows and from each other. A Segment whose transcription failed SHALL render its translation half as an absence, in muted ink, rather than as content.

#### Scenario: Row translated by a fallback backend

- **WHEN** a row was translated by a fallback backend
- **THEN** it carries a visible Degraded mark

#### Scenario: Row whose translation was rejected

- **WHEN** a row's translation failed
- **THEN** the failure is visible on the row rather than an empty translation cell

#### Scenario: Row already in the target language

- **WHEN** a Segment in the target language is displayed
- **THEN** its translation column shows the Segment's text marked as being in the same language, distinct from a pending row and from a translated one

#### Scenario: Row whose transcription failed

- **WHEN** a row's transcription failed
- **THEN** its translation half renders muted as an absence, not as full-emphasis content
