# Tasks

## 1. History keyboard access

- [ ] 1.1 Each history row is focusable, shows a visible focus indicator, and Enter/Space triggers the same open action as click. Acceptance: the full replay flow is reachable keyboard-only.
- [ ] 1.2 History rows set a pointer cursor and a pressed shade.

## 2. Delete confirmation dialog

- [ ] 2.1 The dialog traps Tab within itself, focuses Cancel on open, and Escape dismisses it exactly like the dismiss action. Acceptance: nothing is removed from disk on Escape.
- [ ] 2.2 The Delete button is styled with the danger color so the destructive action reads as dangerous.

## 3. Wizard interaction

- [ ] 3.1 The title field receives focus when the wizard opens; its placeholder no longer duplicates the label.
- [ ] 3.2 Enter in the title field confirms the wizard like the Begin control.
- [ ] 3.3 Wizard validation errors render inside the wizard card under the relevant field, not only in the header status line.
- [ ] 3.4 The Start control is disabled while the wizard is open, so typed input cannot be wiped by reopening; Begin is disabled while a start is in flight and a second start is refused while one is active.

## 4. Contrast and failed-row rendering

- [ ] 4.1 Introduce a secondary-text theme token at >= 4.5:1 contrast on every scheme; reserve faint ink for hairlines and dividers. Acceptance: all text at 11-12px passes WCAG AA on light and dark schemes.
- [ ] 4.2 Darken the light-scheme danger/warning hues to pass 4.5:1 at status-text sizes.
- [ ] 4.3 The translation half of a failed-transcription row renders muted with a 'no translation' style caption instead of a full-emphasis dash.
- [ ] 4.4 Status chips keep stable slots so the header does not reflow on 0-to-N transitions.

## 5. Verification

- [ ] 5.1 Full test suite passes; manual keyboard-only pass: open wizard, start, stop, reopen from history, delete with confirmation.
