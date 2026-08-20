## ADDED Requirements

### Requirement: Wizard keyboard interaction

When the wizard opens, its title field SHALL receive keyboard focus. Pressing Enter in the wizard SHALL confirm it with the same effect as the begin action. Validation failures SHALL be reported inside the wizard card, adjacent to the offending field, in addition to any global status.

#### Scenario: User confirms with Enter

- **WHEN** the wizard is open and the user presses Enter
- **THEN** the Recording begins as if the begin control had been activated

#### Scenario: Validation fails

- **WHEN** the wizard is confirmed with a missing required selection
- **THEN** the validation message appears inside the wizard card and the wizard stays open

### Requirement: Wizard state protection

While the wizard is open, the start control SHALL be disabled so that an in-progress wizard cannot be reset and its typed input lost. While a Recording start is in flight, the begin action SHALL be disabled, and a start SHALL be refused when a Recording is already active.

#### Scenario: User clicks start while the wizard is open

- **WHEN** the wizard is open with a typed title and the start control is activated
- **THEN** the activation has no effect and the wizard keeps its state
