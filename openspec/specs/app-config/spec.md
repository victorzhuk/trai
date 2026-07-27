# app-config Specification

## Purpose
TBD - created by archiving change add-live-transcription-spine. Update Purpose after archive.
## Requirements
### Requirement: TOML configuration file

All configuration SHALL be read from a TOML file. The file SHALL carry the Store root, the default microphone and monitor source names, the whisper server URL, the silence hold, the Segment duration cap, and the confidence floor. The application SHALL NOT provide any interface for editing configuration.

#### Scenario: Application starts with a valid configuration file

- **WHEN** the application starts and the configuration file is present and valid
- **THEN** every configured value is used by the pipeline, with no value hardcoded in its place

#### Scenario: Operator changes a tuning value

- **WHEN** the silence hold or confidence floor is changed in the configuration file and the application is restarted
- **THEN** the new value takes effect without rebuilding

### Requirement: Startup configuration validation

The application SHALL validate configuration at startup and SHALL fail with a message naming the offending key when a required value is missing or invalid, rather than failing later during a Recording.

#### Scenario: Required value missing

- **WHEN** the configuration file omits a required value such as the Store root
- **THEN** the application exits at startup with a message naming that key

#### Scenario: Unreachable whisper server at startup

- **WHEN** the configured whisper server URL is syntactically valid but unreachable
- **THEN** the application reports it at startup rather than silently producing an empty transcript

### Requirement: Translation configuration

The TOML configuration file SHALL carry the target language and the definition of a translate backend, comprising its base URL, its model, and an optional API key. Startup validation SHALL cover these values.

#### Scenario: Target language missing from configuration

- **WHEN** the application starts with no target language configured
- **THEN** it exits at startup naming the missing key, rather than starting a Recording that cannot translate

#### Scenario: Operator points the application at a different backend

- **WHEN** the backend's base URL or model is changed in the configuration file and the application is restarted
- **THEN** translation requests go to the new backend without rebuilding

### Requirement: Ordered translate backend configuration

The TOML configuration file SHALL define translate backends as an ordered list, each entry carrying a base URL, a model, and an optional API key, together with a per-request timeout and a re-probe interval. Startup validation SHALL reject an empty list.

#### Scenario: Configuration defines several backends

- **WHEN** the configuration file lists more than one translate backend
- **THEN** their order in the file is the priority order used for translation and for re-probing

#### Scenario: Configuration defines no backends

- **WHEN** the translate backend list is empty or absent
- **THEN** the application exits at startup naming that key

#### Scenario: Operator adjusts the timeout

- **WHEN** the per-request timeout is changed and the application is restarted
- **THEN** failover triggers at the new threshold without rebuilding

