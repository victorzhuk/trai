## ADDED Requirements

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
