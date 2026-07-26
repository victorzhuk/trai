## ADDED Requirements

### Requirement: Translation panel

The window SHALL present a second panel beside the original-transcription panel, showing each Segment's translation into the configured target language.

#### Scenario: Window is open

- **WHEN** the application window is displayed at its default size
- **THEN** both the original-transcription panel and the translation panel are visible

### Requirement: Aligned rows with pending placeholders

When a Segment closes, both panels SHALL receive a row at the same index. The translation panel's row SHALL show a pending state until that Segment's translation returns, at which point it SHALL be replaced in place.

#### Scenario: Segment closes before its translation returns

- **WHEN** a Segment's row appears in the original panel
- **THEN** a row appears at the same index in the translation panel showing a pending state

#### Scenario: Translation returns

- **WHEN** a translation returns for a Segment whose row is pending
- **THEN** that row's content is replaced in place, with no row appended and no row reordered

#### Scenario: A later Segment is translated first

- **WHEN** a Segment's translation returns after that of a later-starting Segment
- **THEN** each translation lands in its own row and the panels remain aligned row for row

### Requirement: Locked scrolling between panels

The two panels SHALL scroll together so that a row and its translation stay side by side.

#### Scenario: User scrolls one panel

- **WHEN** the user scrolls either panel
- **THEN** the other panel moves to show the same rows
