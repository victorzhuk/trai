## 1. Source enumeration, test-first

- [ ] 1.1 Failing test: parsing a captured source listing yields each source's identifier and human-readable name, and separates inputs from monitors. Acceptance: a monitor source is never offered as a microphone.
- [ ] 1.2 Implement source enumeration and parsing until 1.1 passes.
- [ ] 1.3 Failing test: a configured default that is absent from the listing yields no preselection rather than an error. Acceptance: the remaining sources are still returned.
- [ ] 1.4 Implement default resolution until 1.3 passes.

## 2. Wizard screen, tests after

- [ ] 2.1 Wizard opens on start, with a title prefilled from the current timestamp and editable. Acceptance: accepting the prefilled title requires no typing.
- [ ] 2.2 Microphone and monitor dropdowns populated from live enumeration, preselected to configured defaults. Acceptance: freshly plugged hardware appears without restarting the application.
- [ ] 2.3 Store root displayed read-only. Acceptance: no control offers to change it.
- [ ] 2.4 Start begins a Recording bound to the selected sources. Acceptance: the configuration file is unchanged afterwards.

## 3. Recording metadata

- [ ] 3.1 Persist the chosen title and the selected source names with the Recording. Acceptance: both are readable from the Recording's directory after it ends.

## 4. Verification

- [ ] 4.1 Start a Recording from the wizard with defaults accepted. Acceptance: transcription behaves exactly as before the wizard existed.
- [ ] 4.2 Unplug the default microphone, then open the wizard. Acceptance: the wizard opens, lists what is present, and nothing is preselected for the microphone.
