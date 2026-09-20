# Design

## Context

`WhisperClient` speaks exactly the whisper.cpp server dialect: `POST {base_url}/inference`, multipart with `file`, `response_format=verbose_json`, `temperature=0.0`, `language` always present (`auto` when unconfigured). Config validation already rejects cleartext HTTP off-loopback, and `TranslateBackendConfig` already demonstrates the repo's pattern for an optional bearer key that never reaches logs (manual `Debug`). Grilled decisions, all user-confirmed, are normative for this change:

- Dialect is selected by an explicit `whisper_kind` — never inferred from key presence (PRD story 19: local by default; a stray pasted key must not reroute meeting audio).
- `whisper_api_key` and `whisper_model` are required iff `whisper_kind = "openai"`; their presence under the local dialect is a startup error naming the keys.
- Cloud dialect sends `language` omitted (not `auto`) when detection is wanted; the OpenAI-compatible API rejects `auto`.
- With no per-word probabilities, Segment confidence is `exp(mean avg_logprob)` — the geometric-mean word probability whisper itself optimizes — so one `confidence_floor` governs both dialects.
- Cloud transcription is opt-in, documented, with no runtime notice; the config comment and README carry the privacy warning.

## Goals / Non-Goals

**Goals:**

- One config edit plus a Groq key turns on cloud transcription end-to-end: record → `whisper-large-v3-turbo` → transcript fills.
- A config file written before this change behaves byte-identically to before.
- No secret reaches a log line, and no cloud URL is accepted over cleartext HTTP.

**Non-Goals:**

- Transcriber fallback chains (one Transcribe backend per run; contrast [[Translate backend]]).
- Provider-specific code — no `groq` anywhere; the client speaks the generic OpenAI transcription dialect.
- Word-level timestamps or any new response fields beyond what the confidence filter needs.

## Decisions

**One client struct, two request builders.** `WhisperClient` keeps one `base_url`, timeout, and HTTP client; a `Dialect` enum (`WhisperCpp` | `OpenAi { api_key, model }`) held by the client selects the request shape: URL path, bearer header, `model` field, and whether `language` is omitted. Two builders over the shared multipart body — not two `Transcriber` impls — because response parsing is common: both dialects return `verbose_json` with the same text/language/segments shape.

**Confidence derivation lives in the parser, not the pipeline.** `parse_transcription` already computes mean word probability; when the words array is absent it computes `exp(mean segment avg_logprob)` instead. The pipeline and `confidence_floor` are untouched — the filter sees one `confidence: f64` either way. `exp(avg_logprob)` needs no calibration: `avg_logprob` is the mean token log-probability, so its exponential is the geometric mean probability, the same quantity word probability averages approximate. Typical speech sits at −0.2 to −0.4 (exp ≈ 0.67–0.82); the 0.6 floor separates it from hallucination runs below −0.6 (exp < 0.55) without a new knob.

**Cloud keys are rejected under the local dialect.** Silently ignoring a present `whisper_api_key` would hide a half-finished config switch — exactly the silent-mode-change the explicit `whisper_kind` exists to prevent. Fail loud, name both keys.

**Key secrecy follows the translate pattern.** `Config` keeps the key in a plain `String`; its `Debug` impl (manual, like `TranslateBackendConfig`) renders a placeholder. The debug transcript line logs status, timing, sizes — never headers.

## Risks / Trade-offs

- **`exp(avg_logprob)` is an approximation, not the whisper.cpp word probability.** Dialects may discard slightly different borderline Segments. Accepted: one floor with honest units beats two knobs; the floor is already documented as tunable.
- **Groq rate limits could starve a two-Stream Recording.** A failed request marks the Segment failed, as any transport failure does today; no retry is added. If a real meeting shows starvation, that is new scope.
- **`temperature=0.0` is sent in the openai dialect too.** Both APIs accept it; kept to avoid diverging request builders further.

## Testing

Test-first at the existing seams — `Transcriber` trait and config validation — no network, no UI:

- Config: default dialect when key absent; required-if-openai (missing key, missing model); rejected-under-local (stray key, stray model); unrecognized `whisper_kind` value; key absent from `Debug` output.
- Client (against a local HTTP double at the `Transcriber` seam): openai dialect posts to `/audio/transcriptions` with bearer header, `model`, and no `language` field; whisper.cpp dialect request unchanged (regression: `language=auto` still sent, `/inference` path, no auth header); non-2xx surfaces status + body preview.
- Parser: word-probability mean as today; `exp(mean avg_logprob)` when words absent; mixed segments; empty text.
- Smoke (manual, after green): the Groq curl from the README against a recorded Segment WAV, then one short live Recording with `whisper_kind = "openai"`.

## Implementation plan

Tier **standard**, mode **existing-service-strict**, anchors verified at `252644ab`. Floor: `timeout 600 make check` (`make check` is `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`; the wall clock is there because the Makefile configures no limit and an unlimited run is refused by this project's hooks).

Lenses: `spec`, `quality`, `sec` — the diff carries a bearer credential, its redaction and a non-2xx error path, which is what `sec` reads.

Plan review: **pass** — zarchitect, round 2. Round 1 raised one blocker: task 2.4's constructor cutover had been split across two chunks, leaving `client-request` with a signature its own `make test-unit` could not compile; the cutover and both call-site migrations now sit in `client-request`. Two warnings carried: `config-reject` must seed non-blank companion values (blank-means-absent lands in the next chunk), and task 2.4's work deliberately spans both client chunks, which is a traceability note rather than a hazard.

### Chunks

| # | id | tasks | after | coder | verify |
|---|----|-------|-------|-------|--------|
| 1 | `config-parse` | 1.1, 1.2 | — | rust-coder | `make test-unit && make lint` |
| 2 | `config-reject` | 1.3, 1.4 | `config-parse` (shard `config`) | rust-coder | `make test-unit && make lint` |
| 3 | `config-redact` | 1.5, 1.6 | `config-reject` (shard `config`) | rust-coder | `make test-unit && make lint` |
| 4 | `client-request` | 2.1, 2.2 | `config-parse` | rust-coder | `make test-unit && make lint` |
| 5 | `client-regression` | 2.3, 2.4 | `client-request` | rust-coder | `make test-unit && make lint` |
| 6 | `client-error` | 4.1 | `client-regression` | rust-coder | `make test-unit && make lint` |
| 7 | `confidence-fallback` | 3.1, 3.2 | `client-error` | rust-coder | `make test-unit && make lint` |
| 8 | `confidence-precedence` | 3.3, 3.4 | `confidence-fallback` | rust-coder | `make test-unit && make lint` |
| 9 | `docs` | 5.1, 5.2 | `confidence-precedence` | coder | `make test && make lint` |
| 10 | `docs-changelog` | 5.3 | `docs` | coder | `timeout 600 make check` |

Tasks 6.1 and 6.2 are manual smoke on a live Groq endpoint and a microphone: no chunk owns them, and this run does not tick them.

Chunks 2 and 3 run in the `config` shard worktree, which writes only `src/config.rs`. Chunks 4–8 chain in the integration worktree, which writes `src/transcriber/whisper.rs` and `src/main.rs`. The shard is cut after chunk 1 lands and merged back before chunk 9. The chain is serial because every code chunk edits one of two files; the `Dialect` type is what couples them, and it is created in chunk 1.

### Rulings

- `Dialect` lives in `src/transcriber/whisper.rs` as `pub enum Dialect { WhisperCpp, OpenAi { api_key: String, model: String } }`, `derive(Clone, PartialEq)` plus a manual `fmt::Debug` rendering `api_key` as `<redacted>`; re-exported from `src/transcriber/mod.rs` beside `WhisperClient`. `src/config.rs` imports the vocabulary type only and never names `WhisperClient`.
- `WhisperClient::new(base_url, timeout, dialect) -> Result<Self, TranscribeError>`, with both call sites migrated in the same commit (`src/main.rs` and the whisper timeout test).
- `Config` gains `pub whisper_dialect: Dialect`; `RawConfig` gains the three keys as `Option<String>`; `whisper_kind` is matched against the exact literals `whisper.cpp` / `openai` with no trimming, absent meaning `whisper.cpp`, and an unknown value naming `whisper_kind`.
- Errors are the existing types, and the tests assert their exact `Display` output: `ConfigError::Missing(&'static str)` prints `missing required config key: {key}`, `ConfigError::Invalid { key, reason }` prints `{key}: {reason}`.
- `parse_transcription(body: &str)` keeps its signature and learns nothing about the dialect: any word probability yields the arithmetic word mean as today, otherwise the mean of the carrying segments' `avg_logprob` exponentiated in `f32`, otherwise `0.0`.
- `response_format=verbose_json` and `temperature=0.0` go in both dialects. `language` is always present for `whisper.cpp` (`auto` when unconfigured) and omitted entirely — never the literal `auto` — for `openai`.

### Waivers

Every chunk carries `NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch.` and `NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.` The docs chunks carry the same markers with their own reason: prose only, no behavior to test. A chunk therefore closes against its waiver, and its proof is its own `verify` plus the run's floor — there is no sealed red test to turn green.

Seal note: the kernel seals the test files its pattern matches under `pkgDirs`, which for this repo is `tests/*.rs` — the five pre-existing integration tests. This change adds no integration test file, because every new test belongs in a `#[cfg(test)] mod` beside its module, per repo convention. The seal therefore pins the existing suite against deletion or rewrite and pins none of the new tests; the floor and each chunk's explicit "stays green unmodified" clause carry that weight.

### For an agent running outside this harness

- Working directory is the worktree named in your prompt; assert it with `git -C <worktree> rev-parse --show-toplevel` and never use a relative path that could reach the primary checkout.
- Commit contract: one conventional commit per finished task, subject `≤72` characters, body lines `≤100`, imperative, lowercase; type from `feat fix refactor perf docs test build ci chore revert`. Stage only the files you edited. Never stage `openspec/`.
- Run `make test-unit && make lint` before each commit; the run's floor is `timeout 600 make check`.
- Native tools first: Read / Grep / Glob, never `rg` / `grep` / `cat` / `head` / `tail` / `sed` through a shell. Never re-read a file you already read.

## Plan appendix

```json
{
  "v": 2,
  "change": "add-openai-transcribe-backend",
  "baseSha": "252644aba19d91f93f8c69625206f01716b52d66",
  "generatedAt": "2026-09-20T18:52:00.638Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "sec"
  ],
  "chunks": [
    {
      "id": "config-parse",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "config-dialect",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "local-dialect",
          "cloud-complete",
          "cloud-missing-key",
          "cloud-missing-model",
          "stray-companions",
          "bad-kind",
          "key-redacted"
        ],
        "transitions": [
          {
            "input": "whisper_kind absent, companions absent",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Existing local configuration'; src/config.rs:119 RawConfig Option-key pattern"
          },
          {
            "input": "whisper_kind = \"whisper.cpp\" (exact literal)",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect' treats whisper.cpp and absent identically"
          },
          {
            "input": "whisper_kind = \"openai\" + non-blank whisper_api_key + non-blank whisper_model",
            "state": "cloud-complete",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud dialect configured completely'"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key absent or blank",
            "state": "cloud-missing-key",
            "effect": "forced",
            "evidence": "ConfigError::Missing Display 'missing required config key: {key}' src/config.rs:53; blank-as-absent filter precedent parse_translate_config src/config.rs:220"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key present, whisper_model absent or blank",
            "state": "cloud-missing-model",
            "effect": "forced",
            "evidence": "src/config.rs:53; tasks.md 1.2 acceptance 'both messages name exactly one key'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_api_key present, whisper_model absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_model present, whisper_api_key absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Cloud keys are rejected under the local dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", both companions present",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Fail loud, name both keys'"
          },
          {
            "input": "whisper_kind = any other string, including padded or blank",
            "state": "bad-kind",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Unrecognized dialect'; literal check runs before companion checks so bad kind + stray keys still names whisper_kind"
          },
          {
            "input": "format!(\"{:?}\", config) while cloud-complete",
            "state": "key-redacted",
            "effect": "set",
            "evidence": "manual Debug placeholder \"<redacted>\" pattern src/config.rs:39 (TranslateBackendConfig), applied to Dialect"
          }
        ],
        "forbidden": [
          "dialect inferred from key presence — only the explicit whisper_kind selects (proposal.md 'What Changes'; refusing layer: Config::from_toml_str)",
          "Dialect::OpenAi constructed with a blank api_key or model — blank companions filter to absent first, then fail as Missing (refusing layer: src/config.rs validation)",
          "api key value in any Debug output — refusing layer: manual impl Debug for Dialect in src/transcriber/whisper.rs rendering \"<redacted>\"",
          "a 1.2 error naming more than one key — ConfigError::Missing carries one &'static str and Display names exactly it (refusing layer: src/config.rs:53)",
          "whisper_kind accepted with surrounding whitespace — exact literal match, no trim (refusing layer: from_toml_str literal check)",
          "whisper.cpp dialect coexisting with a surviving cloud companion — startup exits, never warns-and-continues (refusing layer: from_toml_str)"
        ],
        "seeding": [
          "every state seeds through Config::from_toml_str on the existing valid_toml() fixture (tests mod src/config.rs:315) edited by string replacement — the only legal config path; no direct Config struct literals in tests",
          "local-dialect: fixture unchanged; cloud-complete: fixture + whisper_kind = \"openai\" + whisper_api_key = \"test-key\" + whisper_model = \"whisper-large-v3-turbo\"; cloud-missing-key: cloud-complete minus whisper_api_key; cloud-missing-model: cloud-complete minus whisper_model; stray-companions: fixture plus one or both companions; bad-kind: fixture + whisper_kind = \"something-else\"; key-redacted: Debug-format the cloud-complete Config"
        ],
        "budgets": [
          "validation runs exactly once at startup via Config::load → from_toml_str (src/config.rs:232); no runtime re-read",
          "0 retries on transcription requests; a failed request marks the Segment failed (design.md Risks) — no retry knob added",
          "one timeout per request, whisper_timeout_ms, > 0 enforced at src/config.rs:248-250, shared by both dialects"
        ]
      },
      "sites": [
        {
          "task": "1.1",
          "file": "src/config.rs",
          "symbol": "Config::from_toml_str",
          "anchor": "fn absent_optional_language_and_api_key_fields_default_to_none() {",
          "change": "add the default-dialect test beside the existing optional-key tests"
        },
        {
          "task": "1.2",
          "file": "src/config.rs",
          "symbol": "Config::from_toml_str",
          "anchor": "fn missing_required_key_names_that_key_in_the_error() {",
          "change": "add the required-if-openai tests for whisper_api_key and whisper_model"
        },
        {
          "task": "1.1",
          "file": "src/config.rs",
          "symbol": "Config",
          "anchor": "pub struct Config {",
          "change": "add whisper_dialect: Dialect beside whisper_url"
        },
        {
          "task": "1.2",
          "file": "src/transcriber/whisper.rs",
          "symbol": "Dialect",
          "anchor": "pub struct WhisperClient",
          "change": "declare the Dialect enum above WhisperClient, derive(Clone, PartialEq) plus a manual fmt::Debug rendering api_key as <redacted>"
        },
        {
          "task": "1.2",
          "file": "src/transcriber/mod.rs",
          "symbol": "transcriber::Dialect",
          "anchor": "pub use whisper::WhisperClient;",
          "change": "re-export Dialect beside WhisperClient"
        }
      ],
      "codeTasks": [
        "test whisper_kind_absent_defaults_to_whisper_cpp_dialect (task 1.1): valid_toml() parses and config.whisper_dialect == Dialect::WhisperCpp; every existing fixture test stays green unmodified",
        "test openai_without_api_key_names_exactly_whisper_api_key and openai_without_model_names_exactly_whisper_model (task 1.2): err.to_string() is exactly \"missing required config key: whisper_api_key\" / \"missing required config key: whisper_model\"",
        "impl (task 1.6, first half): add `pub enum Dialect { WhisperCpp, OpenAi { api_key: String, model: String } }` with derive(Clone, PartialEq) and a manual fmt::Debug rendering api_key as \"<redacted>\", in src/transcriber/whisper.rs above WhisperClient; re-export it from src/transcriber/mod.rs beside WhisperClient",
        "impl (task 1.6): RawConfig gains whisper_kind/whisper_api_key/whisper_model as Option<String>; Config gains pub whisper_dialect: Dialect; from_toml_str matches whisper_kind against the exact literals \"whisper.cpp\"/\"openai\" with no trim (absent → WhisperCpp, unknown → ConfigError naming whisper_kind), and an openai dialect with a missing or blank companion → ConfigError::Missing naming that one key"
      ]
    },
    {
      "id": "config-reject",
      "taskIds": [
        "1.3",
        "1.4"
      ],
      "prev": "config-parse",
      "sharedPkg": "src/config.rs",
      "parallel": false,
      "seam": "config-dialect",
      "shard": "config",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "local-dialect",
          "cloud-complete",
          "cloud-missing-key",
          "cloud-missing-model",
          "stray-companions",
          "bad-kind",
          "key-redacted"
        ],
        "transitions": [
          {
            "input": "whisper_kind absent, companions absent",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Existing local configuration'; src/config.rs:119 RawConfig Option-key pattern"
          },
          {
            "input": "whisper_kind = \"whisper.cpp\" (exact literal)",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect' treats whisper.cpp and absent identically"
          },
          {
            "input": "whisper_kind = \"openai\" + non-blank whisper_api_key + non-blank whisper_model",
            "state": "cloud-complete",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud dialect configured completely'"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key absent or blank",
            "state": "cloud-missing-key",
            "effect": "forced",
            "evidence": "ConfigError::Missing Display 'missing required config key: {key}' src/config.rs:53; blank-as-absent filter precedent parse_translate_config src/config.rs:220"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key present, whisper_model absent or blank",
            "state": "cloud-missing-model",
            "effect": "forced",
            "evidence": "src/config.rs:53; tasks.md 1.2 acceptance 'both messages name exactly one key'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_api_key present, whisper_model absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_model present, whisper_api_key absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Cloud keys are rejected under the local dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", both companions present",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Fail loud, name both keys'"
          },
          {
            "input": "whisper_kind = any other string, including padded or blank",
            "state": "bad-kind",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Unrecognized dialect'; literal check runs before companion checks so bad kind + stray keys still names whisper_kind"
          },
          {
            "input": "format!(\"{:?}\", config) while cloud-complete",
            "state": "key-redacted",
            "effect": "set",
            "evidence": "manual Debug placeholder \"<redacted>\" pattern src/config.rs:39 (TranslateBackendConfig), applied to Dialect"
          }
        ],
        "forbidden": [
          "dialect inferred from key presence — only the explicit whisper_kind selects (proposal.md 'What Changes'; refusing layer: Config::from_toml_str)",
          "Dialect::OpenAi constructed with a blank api_key or model — blank companions filter to absent first, then fail as Missing (refusing layer: src/config.rs validation)",
          "api key value in any Debug output — refusing layer: manual impl Debug for Dialect in src/transcriber/whisper.rs rendering \"<redacted>\"",
          "a 1.2 error naming more than one key — ConfigError::Missing carries one &'static str and Display names exactly it (refusing layer: src/config.rs:53)",
          "whisper_kind accepted with surrounding whitespace — exact literal match, no trim (refusing layer: from_toml_str literal check)",
          "whisper.cpp dialect coexisting with a surviving cloud companion — startup exits, never warns-and-continues (refusing layer: from_toml_str)"
        ],
        "seeding": [
          "every state seeds through Config::from_toml_str on the existing valid_toml() fixture (tests mod src/config.rs:315) edited by string replacement — the only legal config path; no direct Config struct literals in tests",
          "local-dialect: fixture unchanged; cloud-complete: fixture + whisper_kind = \"openai\" + whisper_api_key = \"test-key\" + whisper_model = \"whisper-large-v3-turbo\"; cloud-missing-key: cloud-complete minus whisper_api_key; cloud-missing-model: cloud-complete minus whisper_model; stray-companions: fixture plus one or both companions; bad-kind: fixture + whisper_kind = \"something-else\"; key-redacted: Debug-format the cloud-complete Config"
        ],
        "budgets": [
          "validation runs exactly once at startup via Config::load → from_toml_str (src/config.rs:232); no runtime re-read",
          "0 retries on transcription requests; a failed request marks the Segment failed (design.md Risks) — no retry knob added",
          "one timeout per request, whisper_timeout_ms, > 0 enforced at src/config.rs:248-250, shared by both dialects"
        ]
      },
      "sites": [
        {
          "task": "1.3",
          "file": "src/config.rs",
          "symbol": "Config::from_toml_str",
          "anchor": "fn cleartext_remote_whisper_url_is_rejected_naming_the_key() {",
          "change": "add the stray-companion rejection tests"
        },
        {
          "task": "1.4",
          "file": "src/config.rs",
          "symbol": "ConfigError",
          "anchor": "fn invalid_whisper_url_names_that_key_in_the_error() {",
          "change": "add the unrecognized-whisper_kind test"
        }
      ],
      "codeTasks": [
        "test cloud_keys_under_local_dialect_are_rejected_naming_them (task 1.3): four cases — a stray whisper_api_key alone → \"whisper_api_key: requires whisper_kind = \\\"openai\\\"\"; a stray whisper_model alone → \"whisper_model: requires whisper_kind = \\\"openai\\\"\"; both present → \"whisper_api_key: requires whisper_kind = \\\"openai\\\" (whisper_model is also set)\"; and an explicit whisper_kind = \"whisper.cpp\" with a key → the same message as the stray case",
        "test unrecognized_whisper_kind_names_whisper_kind (task 1.4): err.to_string() is exactly \"whisper_kind: must be \\\"whisper.cpp\\\" or \\\"openai\\\"\"",
        "impl (task 1.6): reject cloud companions under the local dialect; the kind check runs before the companion checks so a bad kind with stray keys still names whisper_kind",
        "seed every 1.3 case with non-blank companion values — blank-means-absent is config-redact's behaviour, and a blank companion here would make these tests flip when that chunk lands"
      ]
    },
    {
      "id": "config-redact",
      "taskIds": [
        "1.5",
        "1.6"
      ],
      "prev": "config-reject",
      "sharedPkg": "src/config.rs",
      "parallel": false,
      "seam": "config-dialect",
      "shard": "config",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "local-dialect",
          "cloud-complete",
          "cloud-missing-key",
          "cloud-missing-model",
          "stray-companions",
          "bad-kind",
          "key-redacted"
        ],
        "transitions": [
          {
            "input": "whisper_kind absent, companions absent",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Existing local configuration'; src/config.rs:119 RawConfig Option-key pattern"
          },
          {
            "input": "whisper_kind = \"whisper.cpp\" (exact literal)",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect' treats whisper.cpp and absent identically"
          },
          {
            "input": "whisper_kind = \"openai\" + non-blank whisper_api_key + non-blank whisper_model",
            "state": "cloud-complete",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud dialect configured completely'"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key absent or blank",
            "state": "cloud-missing-key",
            "effect": "forced",
            "evidence": "ConfigError::Missing Display 'missing required config key: {key}' src/config.rs:53; blank-as-absent filter precedent parse_translate_config src/config.rs:220"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key present, whisper_model absent or blank",
            "state": "cloud-missing-model",
            "effect": "forced",
            "evidence": "src/config.rs:53; tasks.md 1.2 acceptance 'both messages name exactly one key'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_api_key present, whisper_model absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_model present, whisper_api_key absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Cloud keys are rejected under the local dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", both companions present",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Fail loud, name both keys'"
          },
          {
            "input": "whisper_kind = any other string, including padded or blank",
            "state": "bad-kind",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Unrecognized dialect'; literal check runs before companion checks so bad kind + stray keys still names whisper_kind"
          },
          {
            "input": "format!(\"{:?}\", config) while cloud-complete",
            "state": "key-redacted",
            "effect": "set",
            "evidence": "manual Debug placeholder \"<redacted>\" pattern src/config.rs:39 (TranslateBackendConfig), applied to Dialect"
          }
        ],
        "forbidden": [
          "dialect inferred from key presence — only the explicit whisper_kind selects (proposal.md 'What Changes'; refusing layer: Config::from_toml_str)",
          "Dialect::OpenAi constructed with a blank api_key or model — blank companions filter to absent first, then fail as Missing (refusing layer: src/config.rs validation)",
          "api key value in any Debug output — refusing layer: manual impl Debug for Dialect in src/transcriber/whisper.rs rendering \"<redacted>\"",
          "a 1.2 error naming more than one key — ConfigError::Missing carries one &'static str and Display names exactly it (refusing layer: src/config.rs:53)",
          "whisper_kind accepted with surrounding whitespace — exact literal match, no trim (refusing layer: from_toml_str literal check)",
          "whisper.cpp dialect coexisting with a surviving cloud companion — startup exits, never warns-and-continues (refusing layer: from_toml_str)"
        ],
        "seeding": [
          "every state seeds through Config::from_toml_str on the existing valid_toml() fixture (tests mod src/config.rs:315) edited by string replacement — the only legal config path; no direct Config struct literals in tests",
          "local-dialect: fixture unchanged; cloud-complete: fixture + whisper_kind = \"openai\" + whisper_api_key = \"test-key\" + whisper_model = \"whisper-large-v3-turbo\"; cloud-missing-key: cloud-complete minus whisper_api_key; cloud-missing-model: cloud-complete minus whisper_model; stray-companions: fixture plus one or both companions; bad-kind: fixture + whisper_kind = \"something-else\"; key-redacted: Debug-format the cloud-complete Config"
        ],
        "budgets": [
          "validation runs exactly once at startup via Config::load → from_toml_str (src/config.rs:232); no runtime re-read",
          "0 retries on transcription requests; a failed request marks the Segment failed (design.md Risks) — no retry knob added",
          "one timeout per request, whisper_timeout_ms, > 0 enforced at src/config.rs:248-250, shared by both dialects"
        ]
      },
      "sites": [
        {
          "task": "1.5",
          "file": "src/config.rs",
          "symbol": "Config",
          "anchor": "fn backend_debug_output_redacts_the_api_key() {",
          "change": "add the Config Debug redaction test"
        },
        {
          "task": "1.6",
          "file": "src/config.rs",
          "symbol": "Config::from_toml_str",
          "anchor": "let whisper_url = raw.whisper_url.ok_or(ConfigError::Missing(\"whisper_url\"))?;",
          "change": "finish validation and the debug redaction path"
        }
      ],
      "codeTasks": [
        "test config_debug_redacts_whisper_api_key (task 1.5): a cloud-complete config with whisper_api_key = \"super-secret-key\"; format!(\"{config:?}\") contains no part of the key value and does contain \"<redacted>\"",
        "test blank_companions_are_treated_as_absent: a fixture with whisper_api_key = \"   \" parses as WhisperCpp; openai with a blank whisper_model surfaces Missing(\"whisper_model\")",
        "impl (task 1.6): finish the conditional validation — blank companions filter to absent before the missing check — and extend every_field_round_trips_from_toml with an openai fixture asserting Dialect::OpenAi { api_key, model }"
      ]
    },
    {
      "id": "client-request",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "config-parse",
      "sharedPkg": "src/transcriber/whisper.rs",
      "parallel": false,
      "seam": "client-dialect",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "request-whispercpp",
          "request-openai-detect",
          "request-openai-lang",
          "error-non2xx"
        ],
        "transitions": [
          {
            "input": "transcribe(samples, None or blank language) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:45-51 (form + /inference url) and language_param whisper.rs:85-88 returning DETECT \"auto\""
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:48 language text field; trimmed configured value via whisper.rs:87"
          },
          {
            "input": "transcribe(samples, None or blank language) under Dialect::OpenAi",
            "state": "request-openai-detect",
            "effect": "set",
            "evidence": "design.md 'Cloud dialect sends language omitted (not auto)'; specs/meeting-capture 'No source language configured' (omitting the field in the openai dialect)"
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::OpenAi",
            "state": "request-openai-lang",
            "effect": "set",
            "evidence": "specs/meeting-capture 'Source language configured' (carries that language in whichever form the dialect accepts)"
          },
          {
            "input": "any dialect, non-2xx response such as 401 from the cloud endpoint",
            "state": "error-non2xx",
            "effect": "forced",
            "evidence": "src/transcriber/whisper.rs:74-78 existing branch: 'whisper returned {status}: ' + crate::log::preview(&body, 200)"
          }
        ],
        "forbidden": [
          "the literal value auto anywhere in an openai-dialect request — refusing layer: the openai form builder omits the language field entirely",
          "Authorization header on a whisper.cpp-dialect request — refusing layer: dialect branch in WhisperClient (pinned by 2.3)",
          "model field on a whisper.cpp-dialect request — refusing layer: dialect branch",
          "api key or Authorization value inside any TranscribeError message or debug! line — refusing layer: error construction at whisper.rs:57/61/75-78 and the debug! line whisper.rs:63-69 which logs status, timing, sizes only",
          "byte-for-byte whole-body equality assertions in tests — the multipart boundary is random per request (reqwest Form); assert name=\"field\" + value blocks and field absence instead",
          "per-dialect Transcriber impls or a second client struct — design.md 'One client struct, two request builders'"
        ],
        "seeding": [
          "both dialects seed through WhisperClient::new(url, Duration::from_secs(5), dialect) against a TcpListener HTTP double added to the whisper.rs tests module — pattern serve_response at src/translator/openai.rs:167-171, extended to read headers to the blank line, then Content-Length body bytes, capturing the raw request into an Arc<Mutex<Vec<u8>>> before replying 200 with verbose_json; transcribe(&[0i16; 16], language) keeps the WAV deterministic so whole-request !contains(\"auto\") is sound",
          "error-non2xx seeds through the same double replying 'HTTP/1.1 401 Unauthorized' with a JSON error body"
        ],
        "budgets": [
          "0 retries: one request per Segment, a failed request marks the Segment failed (design.md Risks) — no retry loop added",
          "client timeout = whisper_timeout_ms per request (validated > 0 at src/config.rs:248-250), shared by both dialects; the existing silent-server test keeps its 250 ms budget",
          "non-2xx body preview capped at 200 chars (crate::log::preview(&body, 200), whisper.rs:77)",
          "double tests use a 5 s client timeout and join the server handle — TEST_TIMEOUT precedent src/translator/openai.rs:165"
        ]
      },
      "sites": [
        {
          "task": "2.1",
          "file": "src/transcriber/whisper.rs",
          "symbol": "WhisperClient::transcribe",
          "anchor": "fn request_to_a_silent_server_times_out_instead_of_hanging() {",
          "change": "add the HTTP double helper and the openai-dialect request-shape test"
        },
        {
          "task": "2.2",
          "file": "src/transcriber/whisper.rs",
          "symbol": "WhisperClient::transcribe",
          "anchor": "fn absent_configured_language_asks_the_server_to_detect() {",
          "change": "add the openai configured-language test"
        },
        {
          "task": "2.1",
          "file": "src/main.rs",
          "symbol": "WhisperClient::new",
          "anchor": "let cfg_whisper_url = config.whisper_url.clone();",
          "change": "clone config.whisper_dialect beside cfg_whisper_url and pass it through the on_begin_clicked closure to WhisperClient::new"
        },
        {
          "task": "2.1",
          "file": "src/transcriber/whisper.rs",
          "symbol": "WhisperClient::new",
          "anchor": "WhisperClient::new(format!(\"http://{address}\"), Duration::from_millis(250))",
          "change": "migrate this test call to the three-parameter constructor"
        }
      ],
      "codeTasks": [
        "test helper serve_capture(status_line, body) -> (base_url, Arc<Mutex<Vec<u8>>> raw request, JoinHandle) in the whisper.rs tests module: a loopback TcpListener on a thread that reads headers to the blank line, then Content-Length bytes, captures the raw request and replies. The 200 reply body is exactly {\"text\":\" hello\",\"segments\":[]}. This helper is the prerequisite for tasks 2.1, 2.2, 2.3 and 4.1",
        "test openai_dialect_posts_to_audio_transcriptions_with_bearer_and_model_and_no_language (task 2.1): the request line contains \"POST /audio/transcriptions\"; the captured request contains \"Authorization: Bearer test-key\", name=\"model\" with whisper-large-v3-turbo, name=\"response_format\" with verbose_json, name=\"temperature\" with 0.0; it contains no name=\"language\" and no substring auto (all-zero samples keep the WAV bytes deterministic)",
        "test openai_dialect_with_configured_language_sends_it_and_never_auto (task 2.2): transcribe with Some(\" de \") → name=\"language\" carries the trimmed value de, and the request still holds no literal auto",
        "impl (task 2.4, clean cutover in this chunk): WhisperClient gains dialect: Dialect and new(base_url, timeout, dialect) -> Result<Self, TranscribeError>; the OpenAi branch posts to /audio/transcriptions with .bearer_auth(&api_key) and .text(\"model\", &model). Migrate BOTH call sites in the same commit: src/main.rs:393 (clone config.whisper_dialect into the cfg_* block beside cfg_whisper_url, move it into the on_begin_clicked closure, pass it to WhisperClient::new) and the timeout test at src/transcriber/whisper.rs:194 (pass Dialect::WhisperCpp). `make test-unit && make lint` must compile and pass at this chunk"
      ]
    },
    {
      "id": "client-regression",
      "taskIds": [
        "2.3",
        "2.4"
      ],
      "prev": "client-request",
      "sharedPkg": "src/transcriber/whisper.rs",
      "parallel": false,
      "seam": "client-dialect",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "request-whispercpp",
          "request-openai-detect",
          "request-openai-lang",
          "error-non2xx"
        ],
        "transitions": [
          {
            "input": "transcribe(samples, None or blank language) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:45-51 (form + /inference url) and language_param whisper.rs:85-88 returning DETECT \"auto\""
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:48 language text field; trimmed configured value via whisper.rs:87"
          },
          {
            "input": "transcribe(samples, None or blank language) under Dialect::OpenAi",
            "state": "request-openai-detect",
            "effect": "set",
            "evidence": "design.md 'Cloud dialect sends language omitted (not auto)'; specs/meeting-capture 'No source language configured' (omitting the field in the openai dialect)"
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::OpenAi",
            "state": "request-openai-lang",
            "effect": "set",
            "evidence": "specs/meeting-capture 'Source language configured' (carries that language in whichever form the dialect accepts)"
          },
          {
            "input": "any dialect, non-2xx response such as 401 from the cloud endpoint",
            "state": "error-non2xx",
            "effect": "forced",
            "evidence": "src/transcriber/whisper.rs:74-78 existing branch: 'whisper returned {status}: ' + crate::log::preview(&body, 200)"
          }
        ],
        "forbidden": [
          "the literal value auto anywhere in an openai-dialect request — refusing layer: the openai form builder omits the language field entirely",
          "Authorization header on a whisper.cpp-dialect request — refusing layer: dialect branch in WhisperClient (pinned by 2.3)",
          "model field on a whisper.cpp-dialect request — refusing layer: dialect branch",
          "api key or Authorization value inside any TranscribeError message or debug! line — refusing layer: error construction at whisper.rs:57/61/75-78 and the debug! line whisper.rs:63-69 which logs status, timing, sizes only",
          "byte-for-byte whole-body equality assertions in tests — the multipart boundary is random per request (reqwest Form); assert name=\"field\" + value blocks and field absence instead",
          "per-dialect Transcriber impls or a second client struct — design.md 'One client struct, two request builders'"
        ],
        "seeding": [
          "both dialects seed through WhisperClient::new(url, Duration::from_secs(5), dialect) against a TcpListener HTTP double added to the whisper.rs tests module — pattern serve_response at src/translator/openai.rs:167-171, extended to read headers to the blank line, then Content-Length body bytes, capturing the raw request into an Arc<Mutex<Vec<u8>>> before replying 200 with verbose_json; transcribe(&[0i16; 16], language) keeps the WAV deterministic so whole-request !contains(\"auto\") is sound",
          "error-non2xx seeds through the same double replying 'HTTP/1.1 401 Unauthorized' with a JSON error body"
        ],
        "budgets": [
          "0 retries: one request per Segment, a failed request marks the Segment failed (design.md Risks) — no retry loop added",
          "client timeout = whisper_timeout_ms per request (validated > 0 at src/config.rs:248-250), shared by both dialects; the existing silent-server test keeps its 250 ms budget",
          "non-2xx body preview capped at 200 chars (crate::log::preview(&body, 200), whisper.rs:77)",
          "double tests use a 5 s client timeout and join the server handle — TEST_TIMEOUT precedent src/translator/openai.rs:165"
        ]
      },
      "sites": [
        {
          "task": "2.3",
          "file": "src/transcriber/whisper.rs",
          "symbol": "WhisperClient::transcribe",
          "anchor": "const DETECT: &str = \"auto\";",
          "change": "add the whisper.cpp request regression test"
        },
        {
          "task": "2.4",
          "file": "src/transcriber/whisper.rs",
          "symbol": "WhisperClient::transcribe",
          "anchor": "pub struct WhisperClient {",
          "change": "branch the request builder on the dialect held by the client"
        }
      ],
      "codeTasks": [
        "test whisper_cpp_dialect_request_is_unchanged (task 2.3 regression): the request line contains \"POST /inference\"; the request carries no Authorization header; name=\"language\" is present with auto, name=\"temperature\" 0.0, name=\"response_format\" verbose_json and filename=\"segment.wav\" — same path, headers and field set as today, with the random multipart boundary exempt from byte-compatibility",
        "impl (task 2.4): the WhisperCpp branch keeps {base_url}/inference, no auth header, no model field, language always present via language_param; response_format=verbose_json and temperature=0.0 go in both dialects; the OpenAi branch omits the language field entirely when the trimmed language is empty"
      ]
    },
    {
      "id": "client-error",
      "taskIds": [
        "4.1"
      ],
      "prev": "client-regression",
      "sharedPkg": "src/transcriber/whisper.rs",
      "parallel": false,
      "seam": "client-dialect",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "request-whispercpp",
          "request-openai-detect",
          "request-openai-lang",
          "error-non2xx"
        ],
        "transitions": [
          {
            "input": "transcribe(samples, None or blank language) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:45-51 (form + /inference url) and language_param whisper.rs:85-88 returning DETECT \"auto\""
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:48 language text field; trimmed configured value via whisper.rs:87"
          },
          {
            "input": "transcribe(samples, None or blank language) under Dialect::OpenAi",
            "state": "request-openai-detect",
            "effect": "set",
            "evidence": "design.md 'Cloud dialect sends language omitted (not auto)'; specs/meeting-capture 'No source language configured' (omitting the field in the openai dialect)"
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::OpenAi",
            "state": "request-openai-lang",
            "effect": "set",
            "evidence": "specs/meeting-capture 'Source language configured' (carries that language in whichever form the dialect accepts)"
          },
          {
            "input": "any dialect, non-2xx response such as 401 from the cloud endpoint",
            "state": "error-non2xx",
            "effect": "forced",
            "evidence": "src/transcriber/whisper.rs:74-78 existing branch: 'whisper returned {status}: ' + crate::log::preview(&body, 200)"
          }
        ],
        "forbidden": [
          "the literal value auto anywhere in an openai-dialect request — refusing layer: the openai form builder omits the language field entirely",
          "Authorization header on a whisper.cpp-dialect request — refusing layer: dialect branch in WhisperClient (pinned by 2.3)",
          "model field on a whisper.cpp-dialect request — refusing layer: dialect branch",
          "api key or Authorization value inside any TranscribeError message or debug! line — refusing layer: error construction at whisper.rs:57/61/75-78 and the debug! line whisper.rs:63-69 which logs status, timing, sizes only",
          "byte-for-byte whole-body equality assertions in tests — the multipart boundary is random per request (reqwest Form); assert name=\"field\" + value blocks and field absence instead",
          "per-dialect Transcriber impls or a second client struct — design.md 'One client struct, two request builders'"
        ],
        "seeding": [
          "both dialects seed through WhisperClient::new(url, Duration::from_secs(5), dialect) against a TcpListener HTTP double added to the whisper.rs tests module — pattern serve_response at src/translator/openai.rs:167-171, extended to read headers to the blank line, then Content-Length body bytes, capturing the raw request into an Arc<Mutex<Vec<u8>>> before replying 200 with verbose_json; transcribe(&[0i16; 16], language) keeps the WAV deterministic so whole-request !contains(\"auto\") is sound",
          "error-non2xx seeds through the same double replying 'HTTP/1.1 401 Unauthorized' with a JSON error body"
        ],
        "budgets": [
          "0 retries: one request per Segment, a failed request marks the Segment failed (design.md Risks) — no retry loop added",
          "client timeout = whisper_timeout_ms per request (validated > 0 at src/config.rs:248-250), shared by both dialects; the existing silent-server test keeps its 250 ms budget",
          "non-2xx body preview capped at 200 chars (crate::log::preview(&body, 200), whisper.rs:77)",
          "double tests use a 5 s client timeout and join the server handle — TEST_TIMEOUT precedent src/translator/openai.rs:165"
        ]
      },
      "sites": [
        {
          "task": "4.1",
          "file": "src/transcriber/whisper.rs",
          "symbol": "WhisperClient::transcribe",
          "anchor": "fn samples_to_wav_produces_a_parseable_header() {",
          "change": "add the non-2xx cloud response test"
        }
      ],
      "codeTasks": [
        "test non_2xx_cloud_response_surfaces_status_and_preview_without_the_key (task 4.1): the double replies HTTP/1.1 401 Unauthorized with body {\"error\":\"bad key\"}; err.to_string() contains \"whisper returned 401\" and \"bad key\", and contains no part of the api key value",
        "the production branch already formats \"whisper returned {status}: \" plus crate::log::preview(&body, 200) — this task pins it as a regression; change production code only if the test shows the behaviour cannot hold"
      ]
    },
    {
      "id": "confidence-fallback",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": "client-error",
      "sharedPkg": "src/transcriber/whisper.rs",
      "parallel": false,
      "seam": "confidence-fallback",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "conf-words",
          "conf-logprob",
          "conf-zero"
        ],
        "transitions": [
          {
            "input": "verbose_json with at least one words[].probability",
            "state": "conf-words",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:151-158 existing mean-word-probability path; pinned by parses_mean_confidence_from_verbose_json_words (whisper.rs tests mod :186)"
          },
          {
            "input": "verbose_json with no words but segments[].avg_logprob present (e.g. mean -0.4 → 0.6703 ≥ floor 0.6, Segment kept)",
            "state": "conf-logprob",
            "effect": "set",
            "evidence": "design.md 'when the words array is absent it computes exp(mean segment avg_logprob)'; tasks.md 3.1 acceptance; floor consumed at pipeline.rs:431"
          },
          {
            "input": "conf-logprob with mean avg_logprob -0.8 → 0.4493 < floor 0.6 (Segment discarded by the pipeline filter)",
            "state": "conf-logprob",
            "effect": "forced",
            "evidence": "tasks.md 3.2; discard machinery pinned by segment_below_confidence_floor_is_not_translated (src/pipeline.rs:1063) and a_row_below_the_confidence_floor_is_removed_from_the_ui (src/pipeline.rs:1300)"
          },
          {
            "input": "mixed response: one segment with words, another with avg_logprob only",
            "state": "conf-words",
            "effect": "set",
            "evidence": "words-first precedence keeps the 3.3 regression byte-identical; design.md Testing lists 'mixed segments'"
          },
          {
            "input": "no words and no avg_logprob anywhere (e.g. empty segments array)",
            "state": "conf-zero",
            "effect": "no-op",
            "evidence": "missing_words_yields_zero_confidence pins 0.0 today (whisper.rs tests mod :186); src/transcriber/whisper.rs:155-156"
          }
        ],
        "forbidden": [
          "dialect reaching parse_transcription — the signature stays (body: &str); refusing layer: function boundary whisper.rs:145",
          "word probabilities averaged together with avg_logprob in one mean — precedence is total: any words → word mean only",
          "edits to src/pipeline.rs or src/confidence.rs — the floor check sees one mean_confidence f32 either way (proposal.md Impact)",
          "segments lacking avg_logprob counted in the logprob denominator — the mean runs over carrying segments only",
          "a new calibration constant or second floor knob — one confidence_floor governs both dialects (design.md)"
        ],
        "seeding": [
          "all three states seed through parse_transcription(body) with inline verbose_json string literals in the whisper.rs tests module — the existing convention (parses_mean_confidence_from_verbose_json_words); no HTTP, no client construction",
          "the pipeline-level keep/discard needs no new fixture — it is already seeded through Harness::new + FakeTranscriber (src/pipeline.rs:1195) and stays green unmodified"
        ],
        "budgets": [
          "f32 arithmetic with 1e-3 assert tolerance: exp(-0.4) = 0.670320, exp(-0.8) = 0.449329",
          "floor stays the single existing knob confidence_floor ∈ [0.0, 1.0] (src/config.rs:280-282) — no new numeric",
          "fallback is one pass over segments collecting avg_logprob, mirroring the existing probabilities collection (whisper.rs:151-154) — no extra allocation beyond that vec"
        ]
      },
      "sites": [
        {
          "task": "3.1",
          "file": "src/transcriber/whisper.rs",
          "symbol": "parse_transcription",
          "anchor": "fn parses_mean_confidence_from_verbose_json_words() {",
          "change": "add the avg_logprob geometric-mean test"
        },
        {
          "task": "3.2",
          "file": "src/transcriber/whisper.rs",
          "symbol": "parse_transcription",
          "anchor": "fn missing_words_yields_zero_confidence() {",
          "change": "add the hallucination-level avg_logprob test"
        }
      ],
      "codeTasks": [
        "test segments_with_only_avg_logprob_yield_geometric_mean_confidence (task 3.1): two segments carrying avg_logprob -0.3 and -0.5 and no words → (mean_confidence - 0.6703).abs() < 1e-3 and mean_confidence >= 0.6",
        "test hallucination_level_avg_logprob_lands_below_the_floor (task 3.2): segments carrying avg_logprob -0.8 → (mean_confidence - 0.4493).abs() < 1e-3 and mean_confidence < 0.6; the discard itself is already pinned by src/pipeline.rs tests — no pipeline test or edit here",
        "impl (task 3.4): WhisperSegment gains #[serde(default)] avg_logprob: Option<f32>; when no word probabilities exist, parse_transcription averages the avg_logprob of the segments that carry one and returns its exponential — computed in f32 to match Transcription.mean_confidence; nothing else changes"
      ]
    },
    {
      "id": "confidence-precedence",
      "taskIds": [
        "3.3",
        "3.4"
      ],
      "prev": "confidence-fallback",
      "sharedPkg": "src/transcriber/whisper.rs",
      "parallel": false,
      "seam": "confidence-fallback",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust has no test-writer stage — the rust-coder writes the failing tests and the code in one dispatch. NO-TESTER-WAIVER: same reason; there is no separate test-writer or tester stage for this stack.",
      "verify": "make test-unit && make lint",
      "contract": {
        "states": [
          "conf-words",
          "conf-logprob",
          "conf-zero"
        ],
        "transitions": [
          {
            "input": "verbose_json with at least one words[].probability",
            "state": "conf-words",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:151-158 existing mean-word-probability path; pinned by parses_mean_confidence_from_verbose_json_words (whisper.rs tests mod :186)"
          },
          {
            "input": "verbose_json with no words but segments[].avg_logprob present (e.g. mean -0.4 → 0.6703 ≥ floor 0.6, Segment kept)",
            "state": "conf-logprob",
            "effect": "set",
            "evidence": "design.md 'when the words array is absent it computes exp(mean segment avg_logprob)'; tasks.md 3.1 acceptance; floor consumed at pipeline.rs:431"
          },
          {
            "input": "conf-logprob with mean avg_logprob -0.8 → 0.4493 < floor 0.6 (Segment discarded by the pipeline filter)",
            "state": "conf-logprob",
            "effect": "forced",
            "evidence": "tasks.md 3.2; discard machinery pinned by segment_below_confidence_floor_is_not_translated (src/pipeline.rs:1063) and a_row_below_the_confidence_floor_is_removed_from_the_ui (src/pipeline.rs:1300)"
          },
          {
            "input": "mixed response: one segment with words, another with avg_logprob only",
            "state": "conf-words",
            "effect": "set",
            "evidence": "words-first precedence keeps the 3.3 regression byte-identical; design.md Testing lists 'mixed segments'"
          },
          {
            "input": "no words and no avg_logprob anywhere (e.g. empty segments array)",
            "state": "conf-zero",
            "effect": "no-op",
            "evidence": "missing_words_yields_zero_confidence pins 0.0 today (whisper.rs tests mod :186); src/transcriber/whisper.rs:155-156"
          }
        ],
        "forbidden": [
          "dialect reaching parse_transcription — the signature stays (body: &str); refusing layer: function boundary whisper.rs:145",
          "word probabilities averaged together with avg_logprob in one mean — precedence is total: any words → word mean only",
          "edits to src/pipeline.rs or src/confidence.rs — the floor check sees one mean_confidence f32 either way (proposal.md Impact)",
          "segments lacking avg_logprob counted in the logprob denominator — the mean runs over carrying segments only",
          "a new calibration constant or second floor knob — one confidence_floor governs both dialects (design.md)"
        ],
        "seeding": [
          "all three states seed through parse_transcription(body) with inline verbose_json string literals in the whisper.rs tests module — the existing convention (parses_mean_confidence_from_verbose_json_words); no HTTP, no client construction",
          "the pipeline-level keep/discard needs no new fixture — it is already seeded through Harness::new + FakeTranscriber (src/pipeline.rs:1195) and stays green unmodified"
        ],
        "budgets": [
          "f32 arithmetic with 1e-3 assert tolerance: exp(-0.4) = 0.670320, exp(-0.8) = 0.449329",
          "floor stays the single existing knob confidence_floor ∈ [0.0, 1.0] (src/config.rs:280-282) — no new numeric",
          "fallback is one pass over segments collecting avg_logprob, mirroring the existing probabilities collection (whisper.rs:151-154) — no extra allocation beyond that vec"
        ]
      },
      "sites": [
        {
          "task": "3.3",
          "file": "src/transcriber/whisper.rs",
          "symbol": "parse_transcription",
          "anchor": "fn parses_mean_confidence_from_verbose_json_words() {",
          "change": "pin word probability precedence over avg_logprob"
        },
        {
          "task": "3.4",
          "file": "src/transcriber/whisper.rs",
          "symbol": "parse_transcription",
          "anchor": "let probabilities: Vec<f32> = parsed",
          "change": "make the precedence total"
        }
      ],
      "codeTasks": [
        "test words_take_precedence_over_avg_logprob (task 3.3 + the mixed case): one segment with word probabilities 0.9 and 0.8 plus another carrying avg_logprob only → the arithmetic word mean 0.85; the existing tests parses_mean_confidence_from_verbose_json_words and missing_words_yields_zero_confidence stay green unmodified",
        "impl (task 3.4): make the precedence total — any words[].probability anywhere yields the word mean only; otherwise the segment avg_logprob mean, exponentiated in f32; otherwise 0.0. No dialect input reaches parse_transcription, and src/pipeline.rs and src/confidence.rs are not touched"
      ]
    },
    {
      "id": "docs",
      "taskIds": [
        "5.1",
        "5.2"
      ],
      "prev": "confidence-precedence",
      "parallel": false,
      "seam": "docs",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "coder",
      "waiver": "NO-RED-WAIVER: prose only, no behavior to test. NO-TESTER-WAIVER: no test stage exists for a docs-only change.",
      "verify": "make test && make lint",
      "contract": {
        "states": [],
        "transitions": [],
        "forbidden": [
          "a real API key value in any example or README line — placeholder text only (refusing layer: docs chunk review)",
          "cloud keys shown uncommented in config.toml.example — the cloud block stays commented so copying the example never enables cloud routing"
        ],
        "seeding": [
          "no states to seed; 5.3's green run seeds entirely from the already-landed code chunks"
        ],
        "budgets": []
      },
      "sites": [
        {
          "task": "5.1",
          "file": "config.toml.example",
          "symbol": "whisper_kind",
          "anchor": "whisper_timeout_ms = 120000",
          "change": "document whisper_kind and the commented-out cloud block above whisper_url"
        },
        {
          "task": "5.2",
          "file": "README.md",
          "symbol": "Requirements",
          "anchor": "- A Whisper server (e.g., [whisper.cpp server](https://github.com/ggerganov/whisper.cpp))",
          "change": "add the OpenAI-compatible transcription alternative with the Groq setup and the privacy warning"
        },
        {
          "task": "5.2",
          "file": "README.md",
          "symbol": "Getting started",
          "anchor": "# Edit config.toml: set store_root, sources, whisper_url, whisper_timeout_ms,",
          "change": "mention whisper_kind in the config walkthrough line"
        }
      ],
      "codeTasks": [
        "5.1 config.toml.example: document whisper_kind next to whisper_url with the privacy note that the openai dialect sends meeting audio off the machine; show the cloud block commented out — whisper_kind = \"openai\", whisper_url = \"https://api.groq.com/openai/v1\", whisper_model = \"whisper-large-v3-turbo\" and a placeholder comment for whisper_api_key. Never a real key",
        "5.2 README.md: the whisper-server bullet under Requirements gains the OpenAI-compatible alternative with the Groq URL and model and the same privacy warning; the \"Edit config.toml\" line mentions whisper_kind"
      ],
      "sharedPkg": "trai"
    },
    {
      "id": "docs-changelog",
      "taskIds": [
        "5.3"
      ],
      "prev": "docs",
      "sharedPkg": "README.md",
      "parallel": false,
      "seam": "docs",
      "shard": "",
      "pkgDirs": [
        "src",
        "tests"
      ],
      "pkgs": [],
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "coder": "coder",
      "waiver": "NO-RED-WAIVER: prose only, no behavior to test. NO-TESTER-WAIVER: no test stage exists for a docs-only change.",
      "verify": "timeout 600 make check",
      "contract": {
        "states": [],
        "transitions": [],
        "forbidden": [
          "a real API key value in any example or README line — placeholder text only (refusing layer: docs chunk review)",
          "cloud keys shown uncommented in config.toml.example — the cloud block stays commented so copying the example never enables cloud routing"
        ],
        "seeding": [
          "no states to seed; 5.3's green run seeds entirely from the already-landed code chunks"
        ],
        "budgets": []
      },
      "sites": [
        {
          "task": "5.3",
          "file": "CHANGELOG.md",
          "symbol": "Unreleased",
          "anchor": "## [0.1.1] - 2026-07-27",
          "change": "add an ## [Unreleased] section with an ### Added entry"
        }
      ],
      "codeTasks": [
        "CHANGELOG.md: add an \"## [Unreleased]\" section at the top with \"### Added\" — the OpenAI-compatible transcription backend (whisper_kind = \"openai\" with whisper_api_key and whisper_model, local whisper.cpp still the default) and the avg_logprob geometric-mean confidence fallback. Keep a Changelog shape, newest section on top",
        "task 5.3, the gate: make test && make lint — both green"
      ]
    }
  ],
  "seams": [
    {
      "id": "config-dialect",
      "tasks": [
        "1.1",
        "1.2",
        "1.3",
        "1.4",
        "1.5",
        "1.6"
      ],
      "summary": "Config parses whisper_kind/whisper_api_key/whisper_model into a typed dialect held on Config.whisper_dialect. LOCKED: Dialect is defined in src/transcriber/whisper.rs above WhisperClient (whisper.rs:15) as 'pub enum Dialect { WhisperCpp, OpenAi { api_key: String, model: String } }' with derive(Clone, PartialEq) and a manual fmt::Debug rendering api_key as \"<redacted>\"; re-exported from src/transcriber/mod.rs beside WhisperClient (mod.rs:14). config.rs imports the vocabulary type only, never WhisperClient (proposal OVERSIZE note: the Dialect enum is shared by config validation and the request builders). LOCKED: whisper_kind stays a String in RawConfig and is matched against the exact literals \"whisper.cpp\"/\"openai\" (no trim) at parse time inside from_toml_str — the typed Dialect is constructed there; it does not stay a String on Config. LOCKED order of checks: (1) whisper_kind literal (a bad kind wins over stray companions), (2) blank-filter both companions (blank == absent, precedent: translate api_key filter pinned by blank_translate_backend_api_key_is_none), (3) openai: whisper_api_key required before whisper_model; local: any surviving companion is an error naming offenders in whisper_api_key-then-whisper_model order. Error literals the tests assert verbatim: 'missing required config key: whisper_api_key' and 'missing required config key: whisper_model' (ConfigError::Missing, Display src/config.rs:53); stray key only: 'whisper_api_key: requires whisper_kind = \"openai\"'; stray model only: 'whisper_model: requires whisper_kind = \"openai\"'; both: 'whisper_api_key: requires whisper_kind = \"openai\"; whisper_model too'; unknown kind: 'whisper_kind: must be \"whisper.cpp\" or \"openai\"' (ConfigError::Invalid, Display '{key}: {reason}'). Task 1.5 LOCKED: Debug placeholder literal is \"<redacted>\" on struct Dialect (same literal and pattern as TranslateBackendConfig, src/config.rs:39), reached through Config's existing derive(Debug) — Config keeps its derive and needs no manual Debug. NO-RED-WAIVER: Rust has no test-writer stage; one rust-coder dispatch writes tests 1.1-1.5 first and turns them green with 1.6 in the same dispatch. NO-TESTER-WAIVER: same reason — no separate test-writer agent exists for this stack, the coder owns red and green.",
      "contract": {
        "states": [
          "local-dialect",
          "cloud-complete",
          "cloud-missing-key",
          "cloud-missing-model",
          "stray-companions",
          "bad-kind",
          "key-redacted"
        ],
        "transitions": [
          {
            "input": "whisper_kind absent, companions absent",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Existing local configuration'; src/config.rs:119 RawConfig Option-key pattern"
          },
          {
            "input": "whisper_kind = \"whisper.cpp\" (exact literal)",
            "state": "local-dialect",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect' treats whisper.cpp and absent identically"
          },
          {
            "input": "whisper_kind = \"openai\" + non-blank whisper_api_key + non-blank whisper_model",
            "state": "cloud-complete",
            "effect": "set",
            "evidence": "specs/app-config/spec.md 'Cloud dialect configured completely'"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key absent or blank",
            "state": "cloud-missing-key",
            "effect": "forced",
            "evidence": "ConfigError::Missing Display 'missing required config key: {key}' src/config.rs:53; blank-as-absent filter precedent parse_translate_config src/config.rs:220"
          },
          {
            "input": "whisper_kind = \"openai\", whisper_api_key present, whisper_model absent or blank",
            "state": "cloud-missing-model",
            "effect": "forced",
            "evidence": "src/config.rs:53; tasks.md 1.2 acceptance 'both messages name exactly one key'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_api_key present, whisper_model absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Cloud keys without the cloud dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", whisper_model present, whisper_api_key absent",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Cloud keys are rejected under the local dialect'"
          },
          {
            "input": "whisper_kind absent or \"whisper.cpp\", both companions present",
            "state": "stray-companions",
            "effect": "forced",
            "evidence": "design.md 'Fail loud, name both keys'"
          },
          {
            "input": "whisper_kind = any other string, including padded or blank",
            "state": "bad-kind",
            "effect": "forced",
            "evidence": "specs/app-config/spec.md 'Unrecognized dialect'; literal check runs before companion checks so bad kind + stray keys still names whisper_kind"
          },
          {
            "input": "format!(\"{:?}\", config) while cloud-complete",
            "state": "key-redacted",
            "effect": "set",
            "evidence": "manual Debug placeholder \"<redacted>\" pattern src/config.rs:39 (TranslateBackendConfig), applied to Dialect"
          }
        ],
        "forbidden": [
          "dialect inferred from key presence — only the explicit whisper_kind selects (proposal.md 'What Changes'; refusing layer: Config::from_toml_str)",
          "Dialect::OpenAi constructed with a blank api_key or model — blank companions filter to absent first, then fail as Missing (refusing layer: src/config.rs validation)",
          "api key value in any Debug output — refusing layer: manual impl Debug for Dialect in src/transcriber/whisper.rs rendering \"<redacted>\"",
          "a 1.2 error naming more than one key — ConfigError::Missing carries one &'static str and Display names exactly it (refusing layer: src/config.rs:53)",
          "whisper_kind accepted with surrounding whitespace — exact literal match, no trim (refusing layer: from_toml_str literal check)",
          "whisper.cpp dialect coexisting with a surviving cloud companion — startup exits, never warns-and-continues (refusing layer: from_toml_str)"
        ],
        "seeding": [
          "every state seeds through Config::from_toml_str on the existing valid_toml() fixture (tests mod src/config.rs:315) edited by string replacement — the only legal config path; no direct Config struct literals in tests",
          "local-dialect: fixture unchanged; cloud-complete: fixture + whisper_kind = \"openai\" + whisper_api_key = \"test-key\" + whisper_model = \"whisper-large-v3-turbo\"; cloud-missing-key: cloud-complete minus whisper_api_key; cloud-missing-model: cloud-complete minus whisper_model; stray-companions: fixture plus one or both companions; bad-kind: fixture + whisper_kind = \"something-else\"; key-redacted: Debug-format the cloud-complete Config"
        ],
        "budgets": [
          "validation runs exactly once at startup via Config::load → from_toml_str (src/config.rs:232); no runtime re-read",
          "0 retries on transcription requests; a failed request marks the Segment failed (design.md Risks) — no retry knob added",
          "one timeout per request, whisper_timeout_ms, > 0 enforced at src/config.rs:248-250, shared by both dialects"
        ]
      },
      "codeTasks": [
        "test whisper_kind_absent_defaults_to_whisper_cpp_dialect: valid_toml() parses and config.whisper_dialect == Dialect::WhisperCpp; every existing fixture test stays green unmodified (task 1.1)",
        "test openai_without_api_key_names_exactly_whisper_api_key: err.to_string() == \"missing required config key: whisper_api_key\"; sibling openai_without_model_names_exactly_whisper_model == \"missing required config key: whisper_model\" (task 1.2)",
        "test cloud_keys_under_local_dialect_are_rejected_naming_them: four cases — stray key only (contains 'whisper_api_key: requires whisper_kind = \"openai\"'), stray model only (contains 'whisper_model: requires whisper_kind = \"openai\"'), both (contains both key names), explicit whisper_kind = \"whisper.cpp\" + key (task 1.3)",
        "test unrecognized_whisper_kind_names_whisper_kind: err.to_string() == \"whisper_kind: must be \\\"whisper.cpp\\\" or \\\"openai\\\"\" (task 1.4)",
        "test config_debug_redacts_whisper_api_key: cloud-complete config with api_key \"super-secret-key\"; format!(\"{config:?}\") — assert !contains(\"super-secret-key\") and contains(\"<redacted>\") (task 1.5)",
        "test blank_companions_are_treated_as_absent: fixture + whisper_api_key = \"   \" parses as WhisperCpp; openai + blank whisper_model surfaces Missing(\"whisper_model\")",
        "impl: add Dialect (enum + derive(Clone, PartialEq) + manual Debug with \"<redacted>\") to src/transcriber/whisper.rs above WhisperClient; re-export from src/transcriber/mod.rs (task 1.6)",
        "impl: RawConfig gains whisper_kind/whisper_api_key/whisper_model as Option<String> (src/config.rs:119 pattern); Config gains pub whisper_dialect: Dialect beside whisper_url (src/config.rs:13); from_toml_str implements the locked check order and message literals; Config keeps derive(Debug, Clone) (task 1.6)",
        "impl: extend every_field_round_trips_from_toml with an openai fixture asserting Dialect::OpenAi { api_key, model } exact values"
      ]
    },
    {
      "id": "client-dialect",
      "tasks": [
        "2.1",
        "2.2",
        "2.3",
        "2.4",
        "4.1"
      ],
      "summary": "WhisperClient holds the Dialect from the config seam and branches only the request shape; response parsing stays shared. LOCKED constructor: 'pub fn new(base_url: impl Into<String>, timeout: Duration, dialect: Dialect) -> Result<Self, TranscribeError>' — clean cutover, both call sites migrate in this chunk (src/main.rs:393 and the timeout test at whisper.rs:194 passing Dialect::WhisperCpp). LOCKED wiring site: the on_begin_clicked closure in src/main.rs builds nothing — it clones config.whisper_dialect in the cfg_* clone block (src/main.rs:235-240), moves it into the closure beside cfg_whisper_url (main.rs:318), and passes it to WhisperClient::new at main.rs:393; config never names WhisperClient. LOCKED request shapes. whisper.cpp: POST {base_url}/inference, no Authorization, multipart fields file=segment.wav (audio/wav), response_format=verbose_json, temperature=0.0, language=trimmed configured value or auto (language_param, whisper.rs:85). openai: POST {base_url}/audio/transcriptions, header Authorization: Bearer <api_key> via RequestBuilder::bearer_auth, multipart fields file (identical Part), model=<configured>, response_format=verbose_json, temperature=0.0, and language ONLY when a trimmed non-blank source language is configured — omitted otherwise, never the literal auto. Task 4.1 ANSWER: regression test, not new behavior — the non-2xx branch (whisper.rs:74-78) already surfaces 'whisper returned {status}' plus a 200-char body preview and never headers; 4.1 pins it for the openai path with a 401 and adds the key-absence assertion. NO-RED-WAIVER: Rust has no test-writer stage; one rust-coder dispatch writes the HTTP-double tests first and turns them green in the same dispatch. NO-TESTER-WAIVER: same reason — the coder owns both red and green.",
      "contract": {
        "states": [
          "request-whispercpp",
          "request-openai-detect",
          "request-openai-lang",
          "error-non2xx"
        ],
        "transitions": [
          {
            "input": "transcribe(samples, None or blank language) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:45-51 (form + /inference url) and language_param whisper.rs:85-88 returning DETECT \"auto\""
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::WhisperCpp",
            "state": "request-whispercpp",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:48 language text field; trimmed configured value via whisper.rs:87"
          },
          {
            "input": "transcribe(samples, None or blank language) under Dialect::OpenAi",
            "state": "request-openai-detect",
            "effect": "set",
            "evidence": "design.md 'Cloud dialect sends language omitted (not auto)'; specs/meeting-capture 'No source language configured' (omitting the field in the openai dialect)"
          },
          {
            "input": "transcribe(samples, Some(lang)) under Dialect::OpenAi",
            "state": "request-openai-lang",
            "effect": "set",
            "evidence": "specs/meeting-capture 'Source language configured' (carries that language in whichever form the dialect accepts)"
          },
          {
            "input": "any dialect, non-2xx response such as 401 from the cloud endpoint",
            "state": "error-non2xx",
            "effect": "forced",
            "evidence": "src/transcriber/whisper.rs:74-78 existing branch: 'whisper returned {status}: ' + crate::log::preview(&body, 200)"
          }
        ],
        "forbidden": [
          "the literal value auto anywhere in an openai-dialect request — refusing layer: the openai form builder omits the language field entirely",
          "Authorization header on a whisper.cpp-dialect request — refusing layer: dialect branch in WhisperClient (pinned by 2.3)",
          "model field on a whisper.cpp-dialect request — refusing layer: dialect branch",
          "api key or Authorization value inside any TranscribeError message or debug! line — refusing layer: error construction at whisper.rs:57/61/75-78 and the debug! line whisper.rs:63-69 which logs status, timing, sizes only",
          "byte-for-byte whole-body equality assertions in tests — the multipart boundary is random per request (reqwest Form); assert name=\"field\" + value blocks and field absence instead",
          "per-dialect Transcriber impls or a second client struct — design.md 'One client struct, two request builders'"
        ],
        "seeding": [
          "both dialects seed through WhisperClient::new(url, Duration::from_secs(5), dialect) against a TcpListener HTTP double added to the whisper.rs tests module — pattern serve_response at src/translator/openai.rs:167-171, extended to read headers to the blank line, then Content-Length body bytes, capturing the raw request into an Arc<Mutex<Vec<u8>>> before replying 200 with verbose_json; transcribe(&[0i16; 16], language) keeps the WAV deterministic so whole-request !contains(\"auto\") is sound",
          "error-non2xx seeds through the same double replying 'HTTP/1.1 401 Unauthorized' with a JSON error body"
        ],
        "budgets": [
          "0 retries: one request per Segment, a failed request marks the Segment failed (design.md Risks) — no retry loop added",
          "client timeout = whisper_timeout_ms per request (validated > 0 at src/config.rs:248-250), shared by both dialects; the existing silent-server test keeps its 250 ms budget",
          "non-2xx body preview capped at 200 chars (crate::log::preview(&body, 200), whisper.rs:77)",
          "double tests use a 5 s client timeout and join the server handle — TEST_TIMEOUT precedent src/translator/openai.rs:165"
        ]
      },
      "codeTasks": [
        "test helper serve_capture(status_line, body) -> (base_url, Arc<Mutex<Vec<u8>>> of raw request, JoinHandle): TcpListener + thread, reads headers then Content-Length bytes, replies; prerequisite for 2.1-2.3 and 4.1",
        "test openai_dialect_posts_to_audio_transcriptions_with_bearer_and_model_and_no_language (2.1): request line contains 'POST /audio/transcriptions'; captured request contains 'Authorization: Bearer test-key', the model field block name=\"model\" with value whisper-large-v3-turbo, name=\"response_format\" with verbose_json, name=\"temperature\" with 0.0; does NOT contain name=\"language\" nor the substring auto (zero samples make the body deterministic)",
        "test openai_dialect_with_configured_language_sends_it_and_never_auto (2.2): transcribe(.., Some(\" de \")) — contains the language field block name=\"language\" with value de (trimmed); still no literal auto",
        "test whisper_cpp_dialect_request_is_unchanged (2.3 regression): request line contains 'POST /inference'; does NOT contain 'Authorization'; contains name=\"language\" with value auto, name=\"temperature\" with 0.0, name=\"response_format\" with verbose_json, and filename=\"segment.wav\" — same path, headers, and field set as today; the random multipart boundary is exempt from 'byte-compatible'",
        "test non_2xx_cloud_response_surfaces_status_and_preview_without_the_key (4.1 regression): double replies 401 with body '{\"error\":\"bad key\"}'; err.to_string() contains 'whisper returned 401' and 'bad key'; does NOT contain the key value",
        "impl (2.4): WhisperClient gains field dialect: Dialect; new() takes dialect as third parameter; transcribe branches per Dialect — url format!/inference vs /audio/transcriptions, .bearer_auth(&api_key) and .text(\"model\", &model) only for OpenAi, language field always for WhisperCpp via language_param and only-when-trimmed-non-blank for OpenAi; response_format=verbose_json and temperature=0.0 sent in both dialects (design.md)",
        "impl: migrate src/main.rs — clone config.whisper_dialect into the cfg_* block (main.rs:235-240), move it into the on_begin_clicked closure beside cfg_whisper_url (main.rs:318), pass to WhisperClient::new at main.rs:393; update the timeout test call at whisper.rs:194 to pass Dialect::WhisperCpp"
      ]
    },
    {
      "id": "confidence-fallback",
      "tasks": [
        "3.1",
        "3.2",
        "3.3",
        "3.4"
      ],
      "summary": "Parser-only change; DECISION 5 ANSWER: the parser does NOT learn the dialect — the response shape alone decides, because parse_transcription(body: &str) (whisper.rs:145) takes no dialect input and both dialects return the same verbose_json shape; the client chunk needs no parser change and this chunk touches no request code. LOCKED precedence: any words[].probability anywhere → mean word probability (today's path, byte-identical); else segments[].avg_logprob present → mean of the carrying segments' avg_logprob, then exp() in f32 — the geometric-mean word probability; else 0.0. WhisperSegment (whisper.rs:134-136) gains #[serde(default)] avg_logprob: Option<f32>. The pipeline and confidence_floor are untouched — passes_floor (pipeline.rs:431) consumes one f32 either way; the pipeline-level keep/discard is already pinned by existing tests, so 3.1/3.2 assert the parsed value against the 0.6 floor boundary at the parser seam and lean on those pipeline tests for the discard itself. NO-RED-WAIVER: Rust has no test-writer stage; one rust-coder dispatch writes the parser tests first and turns them green in the same dispatch. NO-TESTER-WAIVER: same reason — the coder owns both red and green.",
      "contract": {
        "states": [
          "conf-words",
          "conf-logprob",
          "conf-zero"
        ],
        "transitions": [
          {
            "input": "verbose_json with at least one words[].probability",
            "state": "conf-words",
            "effect": "set",
            "evidence": "src/transcriber/whisper.rs:151-158 existing mean-word-probability path; pinned by parses_mean_confidence_from_verbose_json_words (whisper.rs tests mod :186)"
          },
          {
            "input": "verbose_json with no words but segments[].avg_logprob present (e.g. mean -0.4 → 0.6703 ≥ floor 0.6, Segment kept)",
            "state": "conf-logprob",
            "effect": "set",
            "evidence": "design.md 'when the words array is absent it computes exp(mean segment avg_logprob)'; tasks.md 3.1 acceptance; floor consumed at pipeline.rs:431"
          },
          {
            "input": "conf-logprob with mean avg_logprob -0.8 → 0.4493 < floor 0.6 (Segment discarded by the pipeline filter)",
            "state": "conf-logprob",
            "effect": "forced",
            "evidence": "tasks.md 3.2; discard machinery pinned by segment_below_confidence_floor_is_not_translated (src/pipeline.rs:1063) and a_row_below_the_confidence_floor_is_removed_from_the_ui (src/pipeline.rs:1300)"
          },
          {
            "input": "mixed response: one segment with words, another with avg_logprob only",
            "state": "conf-words",
            "effect": "set",
            "evidence": "words-first precedence keeps the 3.3 regression byte-identical; design.md Testing lists 'mixed segments'"
          },
          {
            "input": "no words and no avg_logprob anywhere (e.g. empty segments array)",
            "state": "conf-zero",
            "effect": "no-op",
            "evidence": "missing_words_yields_zero_confidence pins 0.0 today (whisper.rs tests mod :186); src/transcriber/whisper.rs:155-156"
          }
        ],
        "forbidden": [
          "dialect reaching parse_transcription — the signature stays (body: &str); refusing layer: function boundary whisper.rs:145",
          "word probabilities averaged together with avg_logprob in one mean — precedence is total: any words → word mean only",
          "edits to src/pipeline.rs or src/confidence.rs — the floor check sees one mean_confidence f32 either way (proposal.md Impact)",
          "segments lacking avg_logprob counted in the logprob denominator — the mean runs over carrying segments only",
          "a new calibration constant or second floor knob — one confidence_floor governs both dialects (design.md)"
        ],
        "seeding": [
          "all three states seed through parse_transcription(body) with inline verbose_json string literals in the whisper.rs tests module — the existing convention (parses_mean_confidence_from_verbose_json_words); no HTTP, no client construction",
          "the pipeline-level keep/discard needs no new fixture — it is already seeded through Harness::new + FakeTranscriber (src/pipeline.rs:1195) and stays green unmodified"
        ],
        "budgets": [
          "f32 arithmetic with 1e-3 assert tolerance: exp(-0.4) = 0.670320, exp(-0.8) = 0.449329",
          "floor stays the single existing knob confidence_floor ∈ [0.0, 1.0] (src/config.rs:280-282) — no new numeric",
          "fallback is one pass over segments collecting avg_logprob, mirroring the existing probabilities collection (whisper.rs:151-154) — no extra allocation beyond that vec"
        ]
      },
      "codeTasks": [
        "test segments_with_only_avg_logprob_yield_geometric_mean_confidence (3.1): two segments avg_logprob -0.3 and -0.5, no words → assert (mean_confidence - 0.6703).abs() < 1e-3 and mean_confidence >= 0.6",
        "test hallucination_level_avg_logprob_lands_below_the_floor (3.2): segments with avg_logprob -0.8 → assert (mean_confidence - 0.4493).abs() < 1e-3 and mean_confidence < 0.6; the discard itself is already pinned at src/pipeline.rs:1063 and :1300 — no pipeline test or edit here",
        "test words_take_precedence_over_avg_logprob (3.3 + mixed): one segment with words probabilities 0.9/0.8, another with avg_logprob only → mean of the words, exactly today's 0.85; existing tests parses_mean_confidence_from_verbose_json_words and missing_words_yields_zero_confidence stay green unmodified (3.3 regression)",
        "impl (3.4): WhisperSegment gains #[serde(default)] avg_logprob: Option<f32> (whisper.rs:134-136); parse_transcription: probabilities non-empty → unchanged word mean; else collect segment avg_logprobs → non-empty → mean then f32 exp(); else 0.0"
      ]
    },
    {
      "id": "docs",
      "tasks": [
        "5.1",
        "5.2",
        "5.3"
      ],
      "summary": "Agreed: mechanical prose with a closed file list — config.toml.example, README.md, CHANGELOG.md — no behavior, so no transitions. NO-RED-WAIVER: Rust has no test-writer stage; the same coder dispatch that edits the docs runs the 5.3 gate. NO-TESTER-WAIVER: same reason — no separate test-writer agent exists for this stack.",
      "contract": {
        "states": [],
        "transitions": [],
        "forbidden": [
          "a real API key value in any example or README line — placeholder text only (refusing layer: docs chunk review)",
          "cloud keys shown uncommented in config.toml.example — the cloud block stays commented so copying the example never enables cloud routing"
        ],
        "seeding": [
          "no states to seed; 5.3's green run seeds entirely from the already-landed code chunks"
        ],
        "budgets": []
      },
      "codeTasks": [
        "5.1 config.toml.example: document whisper_kind above whisper_url with the privacy note that the openai dialect sends meeting audio off-machine; show the cloud block commented-out — whisper_kind = \"openai\", whisper_url = \"https://api.groq.com/openai/v1\", whisper_model = \"whisper-large-v3-turbo\", # whisper_api_key = \"your-api-key-here\" — never a real key",
        "5.2 README.md: the whisper-server prerequisite (README.md:33) gains the OpenAI-compatible alternative with the Groq setup (URL, key, model) and the same privacy warning; the config walkthrough line (README.md:49) mentions whisper_kind",
        "CHANGELOG.md: add a '## [Unreleased]' section on top with '### Added' — OpenAI-compatible transcription backend via whisper_kind = \"openai\" (whisper_api_key + whisper_model) and the avg_logprob geometric-mean confidence fallback; hand-maintained Keep a Changelog file, newest section on top, no compare links in the file today",
        "5.3 run the gate: make test && make lint — both green"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "The TOML configuration file SHALL carry a `whisper_kind` selecting the transcription dialect: `whisper.cpp` (default when the key is absent) or `openai`.",
      "tests": [
        "config::tests::whisper_kind_absent_defaults_to_whisper_cpp_dialect"
      ]
    },
    {
      "shall": "The dialect SHALL be selected by this key alone — never inferred from the presence of credentials — so that adding a key by itself cannot change where meeting audio is sent.",
      "tests": [
        "config::tests::cloud_keys_under_local_dialect_are_rejected_naming_them"
      ]
    },
    {
      "shall": "Startup validation SHALL name the offending key on any dialect-related misconfiguration.",
      "tests": [
        "config::tests::unrecognized_whisper_kind_names_whisper_kind",
        "config::tests::cloud_keys_under_local_dialect_are_rejected_naming_them"
      ]
    },
    {
      "shall": "When `whisper_kind = \"openai\"`, the configuration SHALL carry `whisper_api_key` and `whisper_model`, both required.",
      "tests": [
        "config::tests::openai_without_api_key_names_exactly_whisper_api_key",
        "config::tests::openai_without_model_names_exactly_whisper_model"
      ]
    },
    {
      "shall": "Startup validation SHALL name the missing key when either is absent.",
      "tests": [
        "config::tests::openai_without_api_key_names_exactly_whisper_api_key",
        "config::tests::openai_without_model_names_exactly_whisper_model"
      ]
    },
    {
      "shall": "The key SHALL be sent as a bearer credential on every transcription request and SHALL NOT appear in logs.",
      "tests": [
        "transcriber::whisper::tests::openai_dialect_posts_to_audio_transcriptions_with_bearer_and_model_and_no_language",
        "config::tests::config_debug_redacts_whisper_api_key",
        "transcriber::whisper::tests::non_2xx_cloud_response_surfaces_status_and_preview_without_the_key"
      ]
    },
    {
      "shall": "Each closed Segment SHALL be submitted to the configured Transcribe backend as a single request, in the dialect selected by configuration.",
      "tests": [
        "transcriber::whisper::tests::openai_dialect_posts_to_audio_transcriptions_with_bearer_and_model_and_no_language",
        "transcriber::whisper::tests::whisper_cpp_dialect_request_is_unchanged"
      ]
    },
    {
      "shall": "The request SHALL ask for a response format that carries per-segment confidence.",
      "tests": [
        "transcriber::whisper::tests::openai_dialect_posts_to_audio_transcriptions_with_bearer_and_model_and_no_language"
      ]
    },
    {
      "shall": "When a source language is configured for a Stream, that language SHALL be sent with the request in whichever form the dialect accepts; when none is configured, the request SHALL let the server detect the language in the dialect's own way.",
      "tests": [
        "transcriber::whisper::tests::openai_dialect_with_configured_language_sends_it_and_never_auto",
        "transcriber::whisper::tests::whisper_cpp_dialect_request_is_unchanged"
      ]
    },
    {
      "shall": "Requests SHALL be submitted as Segments close, without an application-side queue.",
      "tests": [
        "src/pipeline.rs::a_row_reaches_the_ui_before_its_transcription_and_is_filled_in_place"
      ]
    },
    {
      "shall": "A transcribed Segment whose confidence falls below the configured floor SHALL be discarded: it SHALL NOT appear in the transcript and SHALL NOT be persisted.",
      "tests": [
        "transcriber::whisper::tests::hallucination_level_avg_logprob_lands_below_the_floor",
        "src/pipeline.rs::a_row_below_the_confidence_floor_is_removed_from_the_ui"
      ]
    },
    {
      "shall": "Confidence SHALL be derived from per-word probabilities when the response provides them; otherwise from the exponential of the mean per-segment log-probability — the geometric-mean word probability — so the same `confidence_floor` value governs both dialects.",
      "tests": [
        "transcriber::whisper::tests::segments_with_only_avg_logprob_yield_geometric_mean_confidence",
        "transcriber::whisper::tests::words_take_precedence_over_avg_logprob"
      ]
    }
  ],
  "testHarness": [
    "valid_toml — src/config.rs:319 — builds a complete valid TOML configuration string fixture",
    "remove_required_key — src/config.rs:703 — strips a key from a TOML string to test missing-key validation",
    "samples_to_wav — src/transcriber/whisper.rs:95 — encodes 16kHz mono i16 PCM samples as in-memory WAV byte vector",
    "silent_server_double — src/transcriber/whisper.rs:191 — binds loopback TcpListener without accepting to test timeout",
    "FakeTranscriber::new — src/transcriber/fake.rs:16 — builds thread-safe FakeTranscriber with expectation registry",
    "FakeTranscriber::expect_call — src/transcriber/fake.rs:26 — registers expected sample buffer and returns rendezvous CallHandle",
    "CallHandle::respond — src/transcriber/fake.rs:59 — completes transcribe call rendezvous with Ok(Transcription)",
    "CallHandle::fail — src/transcriber/fake.rs:63 — completes transcribe call rendezvous with Err(TranscribeError)",
    "transcript_lands_in_start_ms_order — tests/pipeline.rs:10 — integration pipeline test standing up FakeTranscriber, FakeTranslator, and JSONL Store",
    "unreachable_addr — tests/live_backend_smoke.rs:27 — binds and drops loopback TcpListener to produce unreachable URL",
    "lm_studio_backend — tests/live_backend_smoke.rs:34 — builds OpenAITranslator instance pointing to local LM Studio endpoint",
    "serve_sequential_responses — tests/live_backend_smoke.rs:44 — spawns TcpListener server thread replying with sequential canned HTTP responses"
  ],
  "floor": "timeout 600 make check",
  "estimateHours": 6,
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
