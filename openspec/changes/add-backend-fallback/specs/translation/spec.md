## ADDED Requirements

### Requirement: Ordered translate backend list

Translation SHALL be served by the first reachable backend in the configured ordered list. The list SHALL be a priority order, not a pool: requests SHALL NOT be distributed across backends.

#### Scenario: First backend is healthy

- **WHEN** Segments are translated and the first configured backend answers
- **THEN** every request goes to the first backend and no other backend is contacted

#### Scenario: First backend is unreachable at the start of a Recording

- **WHEN** a Recording begins while the first backend refuses connections
- **THEN** translation is served by the next backend in the list

### Requirement: Failover on unavailability

A connection failure, a timeout, or a server-side error against the serving backend SHALL move translation to the next backend in the list. A rejection indicating misconfiguration, such as an unknown model or a refused key, SHALL NOT cause failover.

#### Scenario: Serving backend times out

- **WHEN** a translation request exceeds the configured timeout
- **THEN** translation moves to the next backend and subsequent Segments are served by it

#### Scenario: Serving backend rejects the request as misconfigured

- **WHEN** a backend rejects a request because the configured model is unknown
- **THEN** the affected row reports an error, the backend remains selected, and the next Segment is sent to the same backend

#### Scenario: Every backend fails

- **WHEN** no configured backend can serve a translation
- **THEN** the Segment enters the transcript with its original text and no translation, and transcription is unaffected

### Requirement: Degraded marking

A Segment translated by any backend other than the first in the list SHALL be marked Degraded. The mark SHALL be persisted with the translation so that it survives reopening the Recording.

#### Scenario: Translation served by a fallback backend

- **WHEN** a Segment is translated by a backend other than the first
- **THEN** that Segment is marked Degraded on screen and in the transcript file

#### Scenario: Recording reopened later

- **WHEN** a Recording containing Degraded Segments is reopened
- **THEN** the same Segments are still shown as Degraded

### Requirement: Recovery re-probing

While translation is served by a fallback, the application SHALL periodically probe higher-priority backends with a lightweight request at the configured interval, and SHALL return to the first one that answers. Probing SHALL NOT consume or duplicate a Segment's translation.

#### Scenario: Preferred backend becomes available mid-Recording

- **WHEN** a higher-priority backend answers a probe while a fallback is serving
- **THEN** subsequent Segments are translated by the higher-priority backend and are not marked Degraded

#### Scenario: Probe fails

- **WHEN** a probe to a higher-priority backend fails
- **THEN** the current backend continues serving and no Segment is affected by the probe
