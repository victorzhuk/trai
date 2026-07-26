## Why

The spine takes its two sources from configuration, which means changing headsets requires editing a TOML file and restarting the application — at the exact moment a meeting is starting and there is no time for it. Which microphone is plugged in is the one thing that genuinely varies per meeting, and it is the one thing currently hardest to change.

Recordings also need a name. Without one, history is a list of timestamps.

## What Changes

- Starting a Recording opens a short wizard instead of beginning immediately.
- The wizard prefills a title from the current timestamp and allows editing it.
- It lists the audio sources currently present in the audio graph, with the configured defaults preselected, so the common case is confirming rather than choosing.
- It shows the Store root as read-only, since every Recording goes to the same place.
- Starting applies the selected sources to that Recording only; the configuration file is not modified.
- If a configured default source is no longer present, the wizard still opens with the available sources listed and nothing preselected, rather than failing.

## Capabilities

### New Capabilities

- `recording-wizard`: the pre-Recording screen, its source enumeration, and its defaults

### Modified Capabilities

- `meeting-capture`: a Recording's Streams bind to the sources chosen in the wizard rather than directly to configured names, and a Recording carries a title

## Impact

Replaces the spine's direct start with a wizard step. Adds a dependency on enumerating PipeWire sources at runtime. The Recording's title becomes part of its persisted metadata, which the history change then displays.
