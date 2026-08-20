## ADDED Requirements

### Requirement: Optional per-Stream source language overrides

The per-Stream source language keys SHALL be optional. When a key is absent the Stream's language SHALL be detected; when present it SHALL override detection for that Stream. An absent key SHALL NOT be a startup error.

#### Scenario: Configuration omits both language keys

- **WHEN** the application starts with no per-Stream source language configured
- **THEN** it starts normally and both Streams detect their language

#### Scenario: Configuration pins one Stream's language

- **WHEN** only the microphone Stream's language is configured
- **THEN** that Stream uses the configured language and the monitor Stream detects its own
