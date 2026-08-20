## ADDED Requirements

### Requirement: Keyboard access to history

Every action available on a history row by pointer SHALL also be reachable by keyboard: rows SHALL be focusable with a visible focus indicator, and Enter or Space SHALL perform the same action as a click.

#### Scenario: User opens a past Recording without a pointer

- **WHEN** the user moves focus to a history row and presses Enter
- **THEN** the Recording opens exactly as if the row had been clicked

## MODIFIED Requirements

### Requirement: Deleting a Recording

Deleting a Recording SHALL remove its entire directory, including its audio files, and SHALL require a confirmation that names the Recording being deleted. While the confirmation is open, keyboard focus SHALL stay within it, its cancel action SHALL have initial focus, Escape SHALL dismiss it, and its destructive confirm action SHALL be visually distinct as dangerous.

#### Scenario: User confirms deletion

- **WHEN** the user confirms deletion of a Recording
- **THEN** its directory and all its contents are removed and it disappears from the list

#### Scenario: User dismisses the confirmation

- **WHEN** the user dismisses the deletion confirmation, including by pressing Escape
- **THEN** nothing is removed from disk
