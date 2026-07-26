## ADDED Requirements

### Requirement: Translation configuration

The TOML configuration file SHALL carry the target language and the definition of a translate backend, comprising its base URL, its model, and an optional API key. Startup validation SHALL cover these values.

#### Scenario: Target language missing from configuration

- **WHEN** the application starts with no target language configured
- **THEN** it exits at startup naming the missing key, rather than starting a Recording that cannot translate

#### Scenario: Operator points the application at a different backend

- **WHEN** the backend's base URL or model is changed in the configuration file and the application is restarted
- **THEN** translation requests go to the new backend without rebuilding
