## Why

Transcription is pinned to a local whisper.cpp server speaking `POST /inference`, so an operator with a Groq (or any OpenAI-compatible) transcription key cannot use it without standing up a proxy. PRD story 19 (amended) makes cloud transcription an explicit, key-in-hand opt-in — this change provides that door.

## What Changes

- Config gains `whisper_kind` (`"whisper.cpp"` | `"openai"`, default `"whisper.cpp"`). The explicit field, not key presence, selects the dialect — a stray pasted `whisper_api_key` can never silently reroute meeting audio to a cloud provider.
- When `whisper_kind = "openai"`: `whisper_api_key` and `whisper_model` become required; startup validation names the missing key when either is absent. When `whisper_kind = "whisper.cpp"`: both keys must be absent — a present key or model with the local dialect is a startup error naming the keys, since it signals a misconfigured intent rather than something to ignore.
- Confidence filtering extends to responses without per-word probabilities: when the response carries only segment `avg_logprob`, the Segment's confidence is `exp(mean avg_logprob)` — the geometric-mean word probability — against the unchanged `confidence_floor`.
- `config.toml.example` and README document the cloud option and its privacy trade-off (meeting audio leaves the machine).

## Capabilities

### New Capabilities

### Modified Capabilities

- `app-config`: `whisper_kind` and its conditional-required companions `whisper_api_key`, `whisper_model`
- `meeting-capture`: segment transcription speaks one of two dialects; confidence derivation covers `avg_logprob`-only responses

## Impact

Config surface only for existing users — a config.toml without `whisper_kind` behaves exactly as today. Code: `src/config.rs` (new keys, conditional validation), `src/transcriber/whisper.rs` (dialect branch, language omission, confidence fallback in the response parser). No change to `src/confidence.rs` — the floor check sees one `mean_confidence` value either way. No UI, storage, capture, or translation changes. Cloud usage is HTTPS-only via the existing transport-scheme validation.

OVERSIZE: one feature — the `Dialect` enum is shared by config validation and the request builders, so a split ships the type in one change and its only consumer in the next; ~9 implementation tasks in two files (`src/config.rs`, `src/transcriber/whisper.rs`), the remaining tasks are docs and manual smoke.
