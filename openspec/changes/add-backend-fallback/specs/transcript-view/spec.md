## ADDED Requirements

### Requirement: Degraded and failed rows are visibly distinct

The translation panel SHALL mark rows whose translation came from a fallback backend, and SHALL show an error state on rows whose translation was rejected. Pending, Degraded, and error states SHALL be distinguishable from one another and from a normal row.

#### Scenario: Row translated by a fallback backend

- **WHEN** a Degraded translation is displayed
- **THEN** the row carries a visible mark identifying it as lower-quality output, without changing row alignment between the panels

#### Scenario: Row whose translation was rejected

- **WHEN** a translation is rejected outright
- **THEN** the row shows an error state distinct from both the pending placeholder and the Degraded mark
