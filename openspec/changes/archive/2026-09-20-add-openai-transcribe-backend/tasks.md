# Tasks

Test-first: every failing test below is written and confirmed red before its implementation tasks, per the PRD's testing decisions for the pure core. Verification commands: `cargo test` (resource-limited per project config) and `cargo clippy`.

## 1. Configuration

- [x] 1.1 Failing test: `whisper_kind` absent → config parses with dialect `whisper_cpp`. Acceptance: existing config fixtures parse unchanged.
- [x] 1.2 Failing test: `whisper_kind = "openai"` without `whisper_api_key` → `ConfigError` naming `whisper_api_key`; without `whisper_model` → naming `whisper_model`. Acceptance: both messages name exactly one key.
- [x] 1.3 Failing test: `whisper_kind` absent or `whisper_cpp` with `whisper_api_key` or `whisper_model` present → `ConfigError` naming the offending keys. Acceptance: local-by-default cannot be silently overridden.
- [x] 1.4 Failing test: `whisper_kind = "something-else"` → `ConfigError` naming `whisper_kind`.
- [x] 1.5 Failing test: `format!("{:?}", config)` containing the api key value → test asserts a placeholder is rendered instead. Acceptance: raw key never appears in Debug output.
- [x] 1.6 Implement `whisper_kind` / `whisper_api_key` / `whisper_model` parsing and validation until 1.1–1.5 pass.

## 2. Client dialect

- [x] 2.1 Failing test (HTTP double at the `Transcriber` seam): `openai` dialect posts to `{base_url}/audio/transcriptions` with `Authorization: Bearer <key>`, a `model` field, `response_format=verbose_json`, and NO `language` field when no source language is configured. Acceptance: the double asserts path, header, and exact form fields.
- [x] 2.2 Failing test: `openai` dialect with a configured source language sends that language in the form. Acceptance: dialect-native form, never the literal `auto`.
- [x] 2.3 Regression test: `whisper.cpp` dialect request is byte-compatible with today's — `/inference` path, no auth header, `language=auto` when unconfigured, `temperature=0.0`.
- [x] 2.4 Implement the dialect branch in `WhisperClient` until 2.1–2.3 pass.

## 3. Confidence fallback

- [x] 3.1 Failing test: a `verbose_json` response with segments carrying `avg_logprob` but no word probabilities yields confidence `exp(mean avg_logprob)`. Acceptance: a −0.4 mean gives ≈ 0.67; floor 0.6 keeps the Segment.
- [x] 3.2 Failing test: same shape with mean `avg_logprob` −0.8 (exp ≈ 0.45) is discarded by floor 0.6 in the pipeline filter.
- [x] 3.3 Regression test: responses with word probabilities keep today's mean-word-probability behavior.
- [x] 3.4 Implement the parser fallback until 3.1–3.3 pass.

## 4. Error surfacing

- [x] 4.1 Failing test: a 401 from the cloud endpoint surfaces as a `TranscribeError` carrying the status and body preview, and the key value appears in neither. Acceptance: message names status, not internals or credentials.

## 5. Docs and example

- [x] 5.1 `config.toml.example`: `whisper_kind` documented with the privacy note (cloud sends meeting audio off-machine), cloud keys shown commented-out with Groq example values for URL and model, never a real key.
- [x] 5.2 README: whisper-server prerequisite section gains the OpenAI-compatible alternative with the Groq setup (URL, key, model) and the same privacy warning.
- [x] 5.3 Run `cargo test` and `cargo clippy` — both green.

## 6. Verification (manual)

- [ ] 6.1 Smoke: with `whisper_kind = "openai"` against Groq, record a short spoken sentence on both Streams. Acceptance: both `me` and `them` rows appear with text and detected language, panel content matches `segments.jsonl`.
- [ ] 6.2 With the local server running, remove `whisper_kind` and confirm unchanged local behavior on a short Recording.