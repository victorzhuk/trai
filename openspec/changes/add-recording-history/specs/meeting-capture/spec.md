## ADDED Requirements

### Requirement: Recording finalization on stop

Stopping a Recording SHALL write its metadata to its directory: title, start time, duration, the names of the sources captured, and the target language used for translation.

#### Scenario: Recording is stopped

- **WHEN** the user stops a Recording
- **THEN** its metadata is written to its directory and the Recording is listable in history

#### Scenario: Application is killed before stopping

- **WHEN** the application is killed during a Recording, so that no metadata is written
- **THEN** the Recording's transcript and audio remain on disk and the incomplete directory does not prevent other Recordings from being listed
