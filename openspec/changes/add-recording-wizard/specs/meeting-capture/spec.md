## ADDED Requirements

### Requirement: Streams bind to the sources chosen for the Recording

A Recording's Mic Stream and Monitor Stream SHALL bind to the sources selected for that Recording, rather than directly to the names in configuration. Configured names SHALL serve only as the defaults offered before a Recording starts.

#### Scenario: Recording started with a non-default source

- **WHEN** a Recording is started with a microphone other than the configured default
- **THEN** its Mic Stream captures from the selected source for the whole Recording

### Requirement: Recording title

Every Recording SHALL carry a title, persisted with the Recording.

#### Scenario: Recording ends

- **WHEN** a Recording is stopped
- **THEN** its title and the names of the sources it captured are readable from its directory
