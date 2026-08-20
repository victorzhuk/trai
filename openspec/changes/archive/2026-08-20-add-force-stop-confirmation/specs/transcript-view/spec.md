## MODIFIED Requirements

### Requirement: Recording lifecycle controls

The window SHALL provide controls to start and stop a Recording, and SHALL indicate whether a Recording is currently active. Abandoning in-flight work (force stop) SHALL require a confirmation naming how much work will be discarded whenever such work remains, and SHALL apply immediately when none remains.

#### Scenario: User stops an active Recording

- **WHEN** the user activates the stop control during a Recording
- **THEN** capture ends, the audio files and transcript file are finalized, and the window indicates no Recording is active

#### Scenario: User force-stops with work in flight

- **WHEN** the user activates force stop while Segments are transcribing, translating, or failed
- **THEN** a confirmation appears stating the count of in-flight work, and only confirming it abandons that work

#### Scenario: User dismisses the force-stop confirmation

- **WHEN** the force-stop confirmation is dismissed
- **THEN** the Recording keeps draining and nothing is abandoned

#### Scenario: User force-stops with nothing in flight

- **WHEN** the user activates force stop while no Segment is transcribing or translating
- **THEN** force stop applies immediately with no confirmation
