## ADDED Requirements

### Requirement: Transcription dialect selection

The TOML configuration file SHALL carry a `whisper_kind` selecting the transcription dialect: `whisper.cpp` (default when the key is absent) or `openai`. The dialect SHALL be selected by this key alone — never inferred from the presence of credentials — so that adding a key by itself cannot change where meeting audio is sent. Startup validation SHALL name the offending key on any dialect-related misconfiguration.

#### Scenario: Existing local configuration

- **WHEN** the configuration file omits `whisper_kind`
- **THEN** the application starts in the `whisper.cpp` dialect and behavior is identical to before this key existed

#### Scenario: Cloud keys without the cloud dialect

- **WHEN** the configuration carries `whisper_api_key` or `whisper_model` while `whisper_kind` is `whisper.cpp` or absent
- **THEN** the application exits at startup naming the keys, rather than starting a local Recording with a stray credential

#### Scenario: Unrecognized dialect

- **WHEN** `whisper_kind` is set to any other value
- **THEN** the application exits at startup naming `whisper_kind`

### Requirement: OpenAI transcription backend credentials

When `whisper_kind = "openai"`, the configuration SHALL carry `whisper_api_key` and `whisper_model`, both required. Startup validation SHALL name the missing key when either is absent. The key SHALL be sent as a bearer credential on every transcription request and SHALL NOT appear in logs.

#### Scenario: Cloud dialect missing its model

- **WHEN** the configuration sets `whisper_kind = "openai"` and `whisper_api_key` but omits `whisper_model`
- **THEN** the application exits at startup naming `whisper_model`

#### Scenario: Cloud dialect configured completely

- **WHEN** the configuration sets `whisper_kind = "openai"` with `whisper_url`, `whisper_api_key` and `whisper_model`
- **THEN** the application starts and every transcription request carries the key as a bearer credential and the configured model

#### Scenario: Key value appears in no log line

- **WHEN** the application logs configuration or request failures during a cloud-dialect Recording
- **THEN** the key's value does not appear in any output
