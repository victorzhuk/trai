# recording-wizard Specification

## Purpose
TBD - created by archiving change add-recording-wizard. Update Purpose after archive.

## Requirements

### Requirement: Pre-Recording wizard

Starting a Recording SHALL open a wizard that collects the Recording's title and its two sources before capture begins. The wizard SHALL show the Store root read-only.

#### Scenario: User starts a Recording

- **WHEN** the user activates the start control
- **THEN** the wizard opens showing a title field, a microphone selection, a monitor selection, and the Store root as read-only

#### Scenario: User accepts every default

- **WHEN** the wizard opens with defaults resolved and the user confirms without changing anything
- **THEN** a Recording begins using the prefilled title and the preselected sources

### Requirement: Title prefilled from the current time

The wizard SHALL prefill the title from the current timestamp and SHALL allow the user to edit it.

#### Scenario: User does not type a title

- **WHEN** the user starts a Recording without editing the title
- **THEN** the Recording is titled with the prefilled timestamp

### Requirement: Live source enumeration with configured defaults

The wizard SHALL list the audio sources currently present in the audio graph, distinguishing microphone inputs from output-sink monitors, and SHALL preselect the defaults named in configuration.

#### Scenario: Configured defaults are present

- **WHEN** the wizard opens and both configured default sources exist
- **THEN** both are preselected

#### Scenario: A configured default is absent

- **WHEN** the wizard opens and a configured default source is not present in the audio graph
- **THEN** the wizard still opens, lists the available sources, and leaves that selection empty

#### Scenario: Hardware connected after the application started

- **WHEN** a microphone is connected while the application is running and the wizard is then opened
- **THEN** the new source appears in the list without restarting the application

### Requirement: Selections apply to one Recording only

Source selections made in the wizard SHALL apply to the Recording being started and SHALL NOT modify the configuration file.

#### Scenario: User selects a non-default microphone

- **WHEN** a Recording is started with a microphone other than the configured default
- **THEN** that Recording captures from the selected source and the configuration file is left unchanged

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
