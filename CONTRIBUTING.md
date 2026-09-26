# Contributing to trai

Thanks for your interest. This document covers how to set up a dev
environment, the conventions the codebase already follows, and the
process for proposing a change. Participation is governed by the
[Code of Conduct](.github/CODE_OF_CONDUCT.md). For security reports,
see [SECURITY.md](.github/SECURITY.md).

## Project facts

- **License:** Apache 2.0. By submitting a pull request you agree to
  license your contribution under the same terms. See [LICENSE](LICENSE).
- **Default branch:** `master`. Open PRs against `master`.
- **Toolchain:** A recent stable Rust toolchain (edition 2021). CI
  installs Rustfmt and Clippy via `dtolnay/rust-toolchain@stable`;
  no MSRV is verified.
- **Runtime targets:** Linux with PipeWire for capture; `pactl` for
  the source-selection wizard; a local `whisper.cpp` server or an
  OpenAI-compatible `/v1/audio/transcriptions` endpoint for
  transcription; one or more OpenAI-compatible
  `/v1/chat/completions` endpoints for translation. None of the live
  backends are needed for the default `make test` run.
- **Single crate, not a workspace.** `src/lib.rs` is the library,
  `src/main.rs` is the binary; integration tests live under `tests/`.

## Getting started

### First-time setup

```sh
git clone https://github.com/victorzhuk/trai.git
cd trai

# Linux build deps (Ubuntu/Debian names; the CI apt list is authoritative):
sudo apt install libpipewire-0.3-dev libpulse-dev libdbus-1-dev \
                 libxcb1-dev libxkbcommon-dev libwayland-dev \
                 libegl-dev libgl1-mesa-dev libfontconfig-dev \
                 libfreetype6-dev

# Build and run the test suite
make check
```

`make check` runs `cargo fmt --check`, `cargo clippy --all-targets
-- -D warnings`, and `cargo test` — the same gates CI enforces.

### Useful targets

| Target | What it does |
| --- | --- |
| `make build` | Debug build |
| `make release` | Release build |
| `make run` | Run against `config.toml` (created by `make config`) |
| `make config` | Copy `config.toml.example` to `config.toml` if missing |
| `make fmt` | `cargo fmt` |
| `make fmt-check` | Fail if the tree is unformatted |
| `make lint` | `cargo clippy --all-targets -- -D warnings` |
| `make test` | All tests (unit + integration, no network) |
| `make test-unit` | Library unit tests only |
| `make test-smoke` | Ignored live-backend tests; needs reachable backends |

### Optional tooling

```sh
cargo install cargo-watch
cargo install cargo-nextest
```

## Development workflow

1. **Check for existing issues.** Search GitHub Issues first to avoid
   duplicate work.
2. **Branch off `master`.**
   ```sh
   git checkout master
   git pull
   git checkout -b <type>/<short-description-or-issue-number>
   ```
   Branch prefixes: `feature/`, `fix/`, `refactor/`, `docs/`, `test/`.
3. **Make changes.** Small commits that pass `make check` are easier
   to review than one squashed drop at the end.
4. **Verify locally before pushing.**
   ```sh
   make fmt
   make lint
   make test
   ```
5. **Commit with conventional prefixes.**
   ```
   feat: …      # new user-visible behaviour
   fix: …       # bug fix
   refactor: …  # no user-visible change
   docs: …      # documentation only
   test: …      # test additions or improvements
   chore: …     # tooling, dependencies, non-source changes
   ```
6. **Open a PR.** Use the PR template. Link the issue it closes.
   Respond to review comments with follow-up commits.

## Coding conventions

### Crate layout

```
src/
  capture/         PipeWire capture and WAV writing
  config.rs        TOML config loading and validation
  domain.rs        Segment, SpeakerTag, SegmentState (no external deps)
  history.rs       Recording history listing and replay
  http.rs          HTTP response helpers and body cap
  language.rs      ISO 639-1 canonicalisation
  log.rs           Logging init and text preview
  pipeline.rs      Transcription/translation pipeline core
  recording.rs     Recording lifecycle (start, stop, force-stop)
  scheduler.rs     Translation worker scheduling
  segmenter.rs     Voice activity detection and segmentation
  store.rs         JSONL append-only segment store
  transcript.rs    Row building and status summarisation
  transcriber/     Whisper client + trait + test doubles
  translator/      OpenAI-compatible translator + fallback chain
ui/
  main_window.slint   Slint UI
tests/             Integration tests (gated by `test-doubles` feature)
docs/prd, docs/adr Product / architecture docs
openspec/          Spec-driven change tracking
```

### Rust style

- `cargo fmt` formatting.
- `cargo clippy --all-targets -- -D warnings` clean.
- Prefer small, focused functions; named constants with a comment
  that explains the value rather than magic numbers.
- Use concrete types. Introduce a trait only when there are at least
  two real implementations (production and a test double counts).
- Prefer `Result<T, E>` over panicking; use `?` for propagation.
- Domain types in `src/domain.rs` keep **zero external dependencies**.
- Newtypes for error variants match the surrounding style — the
  codebase uses `String` newtypes with manual `Debug` impls rather
  than pulling in `thiserror` for a one-variant error.
- Manual `Debug` impls exist where `{:?}` would otherwise dump a
  bearer key or raw meeting audio metadata into a log. If your type
  holds anything sensitive, write a redacting `Debug` by hand.

### Documentation

- Public items in `src/lib.rs` and its public modules carry `///`
  doc comments explaining intent, not just restating the signature.
- Comments next to code explain *why*, not *what* — the code
  already shows the what.
- Cross-reference the glossary in [CONTEXT.md](CONTEXT.md) instead
  of redefining terms (Recording, Stream, Speaker tag, Segment, …).

### Tests

- Unit tests live next to the code under `#[cfg(test)] mod tests`.
- Integration tests live under `tests/` and depend on the crate via
  the `test-doubles` feature. The `FakeTranscriber` / `FakeTranslator`
  test doubles in `src/transcriber/fake.rs` and
  `src/translator/fake.rs` are the supported way to control
  completion order without timing.
- The default `make test` does not need a network or a live
  whisper server. Anything that needs a live backend lives behind
  `#[ignore]` and runs only under `make test-smoke`.
- Table-driven tests are preferred for validation logic. Temporary
  directories go through `tempfile::TempDir`.

### Privacy as a design constraint

Meeting audio and transcription text are sensitive by definition.
Treat them accordingly:

- Never log raw audio, raw transcription text, or `whisper_api_key`
  at any verbosity.
- Never write meeting content outside `store_root`.
- When `whisper_kind = "openai"` is set, audio leaves the machine —
  that is documented behaviour, not a bug, but new code must not
  silently bypass it.
- No new network endpoints without an accompanying config knob and
  a `config.toml.example` entry.

## Pull request expectations

A good PR:

- **Does one thing.** If a reviewer can describe it in one
  sentence, the diff is the right size.
- **Updates the docs it changes.** A behaviour change without a
  `README.md` / `docs/prd/` / `CONTEXT.md` update is incomplete. A
  fix that affects user-visible behaviour ships with a
  `CHANGELOG.md` entry under `[Unreleased]`.
- **Does not refactor unrelated code** while it's in the diff.
- **Passes `make check` locally** before requesting review.

Reviewers read top to bottom. If the order of commits matters for
review, say so in the PR description.

## Reporting issues not covered here

Open a GitHub issue. There is no separate forum, chat, or mailing
list for this project.
