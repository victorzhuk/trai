## ADDED Requirements

### Requirement: Whisper request timeout

The TOML configuration file SHALL carry a per-request timeout for the whisper server, and startup validation SHALL reject a missing or non-positive value naming the key. The whisper client SHALL apply the timeout to every request so that a hung server fails the request instead of blocking indefinitely.

#### Scenario: Timeout key missing

- **WHEN** the configuration file omits the whisper request timeout
- **THEN** the application exits at startup with a message naming the key

#### Scenario: Whisper server stops answering

- **WHEN** the whisper server accepts a connection but never responds during a Recording
- **THEN** the request fails after the configured timeout and the Segment is marked failed, and stopping the Recording still completes

### Requirement: Transport scheme validation

Startup validation SHALL reject `http://` URLs for non-loopback hosts on `whisper_url` and on every translate backend base URL, naming the offending key. Loopback hosts (localhost, 127.0.0.1, ::1) MAY use `http://`. Any host MAY use `https://`.

#### Scenario: Cleartext remote whisper URL

- **WHEN** the whisper URL uses `http://` with a non-loopback host
- **THEN** the application exits at startup naming `whisper_url`

#### Scenario: Local whisper server over HTTP

- **WHEN** the whisper URL uses `http://127.0.0.1:8080`
- **THEN** startup validation accepts it

#### Scenario: Cleartext remote translate backend

- **WHEN** a translate backend's base URL uses `http://` with a non-loopback host
- **THEN** the application exits at startup naming that backend's base URL key
