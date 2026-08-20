## Why

The window is mouse-only where it matters most. History rows are TouchArea-only, so reopening a past Recording has no keyboard path. The delete confirmation has no Escape, no initial focus, and no focus trap, and its destructive Delete button is styled identically to Cancel. In the wizard the title field is not focused on open, Enter does not confirm, validation errors surface in the header far from the wizard, and clicking Start while the wizard is open silently wipes the typed title and selections. Several status and caption texts fall below WCAG AA contrast on light schemes.

## What Changes

- History rows are focusable and open on Enter/Space with a visible focus indicator.
- The delete confirmation traps focus, focuses Cancel on open, dismisses on Escape, and styles Delete as a destructive action.
- The wizard focuses its title field on open, confirms on Enter, shows its validation errors inside the wizard card, and cannot be reset by the Start control while open; the Begin control refuses reentry while a start is in flight.
- Text colors meet WCAG AA contrast at the sizes they are used; faint ink is reserved for non-text decoration. The translation half of a failed-transcription row reads as an absence (muted), not content.

## Capabilities

### Modified Capabilities

- `recording-history`: keyboard operability, confirmation dialog focus behavior
- `recording-wizard`: focus, Enter-to-confirm, in-card validation errors, reset protection
- `transcript-view`: start-control reentrancy, text contrast floor, failed-row rendering

## Impact

Pure interaction and visual changes; no configuration, storage format, or pipeline behavior changes. Mouse flows are unchanged.
