# recording-history Specification

## Purpose
TBD - created by archiving change add-recording-history. Update Purpose after archive.

## Requirements

### Requirement: Recording list

The application SHALL present a list of past Recordings found in the Store root, ordered newest first, each showing its title, date, duration, and number of transcript lines.

#### Scenario: Recording is stopped

- **WHEN** a Recording is stopped
- **THEN** it appears at the top of the history list

#### Scenario: Store root contains an incomplete directory

- **WHEN** a directory in the Store root has no readable metadata
- **THEN** it is omitted from the list and the remaining Recordings are still listed

### Requirement: Reopening a Recording

Opening a Recording from history SHALL display its transcript in the same two panels used during a Recording, with input disabled. The display SHALL merge Segment records with their translation records and SHALL preserve Speaker tags and Degraded marks.

#### Scenario: User opens a past Recording

- **WHEN** a Recording is opened from the history list
- **THEN** its transcript is shown in the two panels exactly as it appeared during the meeting, and no capture is started

#### Scenario: Recording contains an untranslated Segment

- **WHEN** a reopened Recording contains a Segment that was never translated
- **THEN** that Segment's original text is shown and its translation row is empty rather than pending

### Requirement: Deleting a Recording

Deleting a Recording SHALL remove its entire directory, including its audio files, and SHALL require a confirmation that names the Recording being deleted. While the confirmation is open, keyboard focus SHALL stay within it, its cancel action SHALL have initial focus, Escape SHALL dismiss it, and its destructive confirm action SHALL be visually distinct as dangerous.

#### Scenario: User confirms deletion

- **WHEN** the user confirms deletion of a Recording
- **THEN** its directory and all its contents are removed and it disappears from the list

#### Scenario: User dismisses the confirmation

- **WHEN** the user dismisses the deletion confirmation, including by pressing Escape
- **THEN** nothing is removed from disk

### Requirement: Keyboard access to history

Every action available on a history row by pointer SHALL also be reachable by keyboard: rows SHALL be focusable with a visible focus indicator, and Enter or Space SHALL perform the same action as a click.

#### Scenario: User opens a past Recording without a pointer

- **WHEN** the user moves focus to a history row and presses Enter
- **THEN** the Recording opens exactly as if the row had been clicked
