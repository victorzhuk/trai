# trai

A local-first meeting transcribe-and-translate app for Linux. Captures both
sides of a call (your microphone and the meeting app's output), transcribes
each side with a local Whisper server, translates into your configured
language, and writes everything to disk as the meeting runs.

## What it does

- **Two streams, one transcript.** Records the microphone (`me`) and the
  output monitor (`them`) as separate PipeWire streams. Segments interleave
  by timestamp so the transcript reads as a conversation.
- **Live panels.** Original text and translation appear side by side as
  people speak. In-flight rows show a spinner; same-language rows skip
  translation and show the original text marked accordingly.
- **Backend fallback.** Translation backends are an ordered list in config.
  The first reachable one wins; on failure the pipeline falls over to the
  next and marks segments **degraded**. Earlier backends are re-probed
  automatically.
- **Language auto-detection.** With no per-stream language configured,
  Whisper detects the source language per segment. Segments already in the
  target language are not translated.
- **Recording history.** Past recordings are listed and reopenable in the
  same two-panel view. Recordings can be deleted behind a confirmation.
- **Crash-safe.** Segments append to `segments.jsonl` live; both streams are
  teed to WAV. A crash costs at most the last line.
- **Force stop.** When translation backends are dead, a **Force stop**
  button detaches translation threads so stop returns immediately.

## Requirements

- Linux with PipeWire, including the `pw-record` tool (on most distros part
  of the `pipewire` package) — recording is done by spawning two
  `pw-record` subprocesses.
- A recent stable Rust toolchain (e.g. via [rustup](https://rustup.rs)) for
  building from source.
- Development packages needed by the build (Debian/Ubuntu names; use the
  equivalents on your distro): `libpipewire-0.3-dev`, `libpulse-dev`,
  `libdbus-1-dev`, `libxcb1-dev`, `libxkbcommon-dev`, `libwayland-dev`,
  `libegl-dev`, `libgl1-mesa-dev`, `libfontconfig-dev`,
  `libfreetype6-dev`.
- A Whisper server (e.g., [whisper.cpp server](https://github.com/ggerganov/whisper.cpp))
  reachable at a configured URL. Alternatively, any OpenAI-compatible
  transcription endpoint works by setting `whisper_kind = "openai"` — e.g.
  [Groq](https://console.groq.com) with `whisper_url =
  "https://api.groq.com/openai/v1"`, `whisper_model =
  "whisper-large-v3-turbo"`, and your `whisper_api_key`. **Privacy warning:**
  unlike a local server, the cloud endpoint sends your meeting audio
  off-machine.
- One or more OpenAI-compatible translation endpoints (e.g., LM Studio,
  Ollama, or any `/v1/chat/completions` server)

## Getting started

```sh
# Clone and build
git clone https://github.com/victorzhuk/trai.git
cd trai
make build

# Create your config from the template
make config
# Edit config.toml: set store_root, sources, whisper_kind, whisper_url,
# whisper_timeout_ms, target_language, and your translation backends

# Run
make run
```

CI on this repository builds a release binary on every push to `master`
and every `v*` tag and uploads it as a GitHub Actions **run artifact**
named `trai-linux-x86_64-${{ github.sha }}`. These are transient
per-run artifacts: retention is 14 days, and downloading from a run's
page is only useful inside that window. The durable per-tag
distribution, when one is cut, is a release asset
(`trai-v0.2.1-linux-x86_64.tar.gz`) attached to the corresponding
GitHub Release after a green tag CI run and a manual curation step.
Tags alone do not create releases — only a manually curated GitHub
Release does, so not every `v*` tag automatically produces a release
asset. Download run artifacts from the run's page under the Actions
tab. The downloaded archive is plain-zipped —
`actions/upload-artifact` strips the executable bit on direct
extraction, so restore it before running. The binary also requires a
config file — either `config.toml` in the working directory or a path
passed as the first CLI argument. Copy `config.toml.example` from the
repository and fill it in first:

```sh
chmod +x trai
./trai /path/to/config.toml
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to participate, and
[SECURITY.md](.github/SECURITY.md) for how to report a vulnerability.
Confidential vulnerability reports should go through GitHub's private
advisory flow at
<https://github.com/victorzhuk/trai/security/advisories/new> — that
is the only confidential channel for security disclosures on this
repository. Public issues are not confidential and may persist in
caches, notifications, and edit history; do not post sensitive data
there. Code-of-conduct and other private conduct matters go to
<dev@victorzh.uk>; that address is not a security channel.

## Configuration

Configuration is a single `config.toml` file. Copy `config.toml.example` and
fill in the placeholders. There is no settings UI.

```toml
store_root = "/home/you/.local/share/trai/recordings"
mic_source = "alsa_input.pci-0000_00_1f.3.analog-stereo"
monitor_source = "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor"

# Optional: force a stream's language instead of auto-detecting.
# mic_language = "en"
# monitor_language = "de"

whisper_url = "http://localhost:9000"
target_language = "en"

# Voice activity detection
vad_threshold = 0.5
silence_hold_ms = 300        # Close a segment after this much silence
duration_cap_ms = 30000      # Hard ceiling on segment duration
live_chunk_ms = 3000         # Cut ongoing speech this often for live text
confidence_floor = 0.6       # Discard segments below this mean probability

[translate]
request_timeout_ms = 60000
reprobe_interval_ms = 30000

[[translate.backends]]
base_url = "https://192.168.1.50:1234"
model = "qwen/qwen3.5-9b"
# api_key = "optional-bearer-token"

[[translate.backends]]
base_url = "http://localhost:11434"
model = "qwen3.5:cloud"
```

Plain `http://` is only accepted for loopback hosts. Remote backends must
use `https://`, since the API key rides every request.

## How it works

1. **Capture.** Two `pw-record` subprocesses read raw 16 kHz mono PCM from
   the microphone and the output monitor.
2. **Segmentation.** Voice activity detection (`earshot`) opens a segment on
   speech onset, closes it after a silence hold, and force-closes at the
   `live_chunk_ms` interval during continuous speech.
3. **Transcription.** Each segment is sent to the Whisper server as a WAV
   multipart request. The detected (or configured) language is recorded on
   the segment.
4. **Translation.** Segments not in the target language are translated via
   the first reachable backend, with the previous three translated lines as
   context. Same-language segments skip translation.
5. **Storage.** Segments append to `segments.jsonl`; audio is teed to
   `mic.wav` and `monitor.wav`. One directory per recording under
   `store_root`.

## Development

```sh
make check     # fmt-check + clippy + tests
make test      # unit + integration tests (no network)
make test-smoke # live-backend smoke tests (needs local backends)
make fmt       # format the tree
make lint      # clippy, warnings as errors
```

## Project layout

```
src/
  capture/     PipeWire capture and WAV writing
  config.rs    TOML config loading and validation
  domain.rs    Segment, SpeakerTag, SegmentState
  history.rs   Recording history listing and replay
  language.rs  ISO 639-1 canonicalisation
  pipeline.rs  Transcription/translation pipeline core
  recording.rs Recording lifecycle (start, stop, force-stop)
  segmenter.rs Voice activity detection and segmentation
  store.rs     JSONL append-only segment store
  transcriber/ Whisper client and trait
  transcript.rs Row building and status summarisation
  translator/  OpenAI-compatible translator, fallback chain
ui/
  main_window.slint  Slint UI
docs/
  prd/        Product requirements
  adr/        Architecture decision records
openspec/     Spec-driven change tracking
```

## License

See [LICENSE](LICENSE).
