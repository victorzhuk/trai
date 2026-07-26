## Why

The primary translate backend runs on a different machine on the LAN. That machine can be switched off, asleep, or unreachable because the service is bound inside WSL behind its own NAT — and the moment it is, the translation panel is dead for the entire meeting, which is when you can least afford to stop and debug networking. A local model on this machine is slower and weaker but always there.

This change makes the translation panel survive the primary backend being gone, and makes the resulting quality drop visible rather than silent.

## What Changes

- Configuration holds an ordered list of translate backends instead of one. The first reachable backend serves translations.
- A transport failure or timeout against the serving backend moves translation to the next backend in the list; if every backend fails, the affected rows stay untranslated and the transcription panel is unaffected.
- Segments translated by any backend other than the first are marked Degraded, both on screen and in the transcript file, so a quality drop is noticed during the meeting rather than discovered afterwards.
- While serving from a fallback, the application periodically re-probes higher-priority backends and returns to the first one that answers.
- A rejection that indicates misconfiguration rather than unavailability — an unknown model, a refused key — surfaces as an error instead of silently demoting the backend.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `translation`: gains the ordered backend list, failover on transport failure, Degraded marking, and recovery re-probing
- `transcript-view`: gains the visible Degraded marking on affected rows
- `app-config`: the single translate backend becomes an ordered list, with a request timeout and a re-probe interval

## Impact

Changes the shape of the translate backend configuration from one entry to a list; a configuration file written for the previous change needs its backend section adjusted. Adds a Degraded flag to persisted translation records, which the history view must display. No change to capture, segmentation, transcription, or the transcript file's append-only property.
