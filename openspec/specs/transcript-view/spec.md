# transcript-view Specification

## Purpose
TBD - created by archiving change add-live-transcription-spine. Update Purpose after archive.
## Requirements
### Requirement: Original transcription panel

The application SHALL present a single window containing a panel that displays the Recording's transcript as it is produced. Each row SHALL show the Segment's Speaker tag, its timestamp, and its transcribed text. Rows SHALL be positioned by Segment start timestamp.

#### Scenario: Speech is transcribed during a Recording

- **WHEN** a Segment is transcribed
- **THEN** a row appears in the panel showing its Speaker tag, timestamp and text

#### Scenario: A late transcription arrives

- **WHEN** a Segment's transcription returns after that of a later-starting Segment
- **THEN** its row is placed at its timestamp position rather than appended at the end

### Requirement: Recording lifecycle controls

The window SHALL provide controls to start and stop a Recording, and SHALL indicate whether a Recording is currently active.

#### Scenario: User stops an active Recording

- **WHEN** the user activates the stop control during a Recording
- **THEN** capture ends, the audio files and transcript file are finalized, and the window indicates no Recording is active

### Requirement: Translation panel

The window SHALL present a second panel beside the original-transcription panel, showing each Segment's translation into the configured target language.

#### Scenario: Window is open

- **WHEN** the application window is displayed at its default size
- **THEN** both the original-transcription panel and the translation panel are visible

### Requirement: Aligned rows with pending placeholders

When a Segment closes, both panels SHALL receive a row at the same index. The translation panel's row SHALL show a pending state until that Segment's translation returns, at which point it SHALL be replaced in place.

#### Scenario: Segment closes before its translation returns

- **WHEN** a Segment's row appears in the original panel
- **THEN** a row appears at the same index in the translation panel showing a pending state

#### Scenario: Translation returns

- **WHEN** a translation returns for a Segment whose row is pending
- **THEN** that row's content is replaced in place, with no row appended and no row reordered

#### Scenario: A later Segment is translated first

- **WHEN** a Segment's translation returns after that of a later-starting Segment
- **THEN** each translation lands in its own row and the panels remain aligned row for row

### Requirement: Locked scrolling between panels

The translation column SHALL share the original panel's scrolling container, so a row and its translation scroll as one and can never drift apart.

#### Scenario: User scrolls the transcript view

- **WHEN** the user scrolls the transcript view
- **THEN** the original and translation columns move together, keeping each row beside its translation

### Requirement: Degraded and failed rows are visibly distinct

The translation panel SHALL mark rows whose translation came from a fallback backend, and SHALL show an error state on rows whose translation was rejected. Pending, Degraded, and error states SHALL be distinguishable from one another and from a normal row.

#### Scenario: Row translated by a fallback backend

- **WHEN** a Degraded translation is displayed
- **THEN** the row carries a visible mark identifying it as lower-quality output, without changing row alignment between the panels

#### Scenario: Row whose translation was rejected

- **WHEN** a translation is rejected outright
- **THEN** the row shows an error state distinct from both the pending placeholder and the Degraded mark

