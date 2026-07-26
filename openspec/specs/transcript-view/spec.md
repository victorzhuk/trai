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

