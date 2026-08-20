# Tasks

## 1. Whisper request timeout

- [x] 1.1 Failing test: configuration without `whisper_timeout_ms` fails at load naming the key; a non-positive value fails the same way. Acceptance: the error message names `whisper_timeout_ms`.
- [x] 1.2 Failing test: the whisper client is built with the configured timeout. Acceptance: a request to a server that never answers returns a timeout error within the configured bound, not a hang.
- [x] 1.3 Implement until 1.1 and 1.2 pass; example configuration documents the new key.

## 2. Transport scheme validation

- [x] 2.1 Failing test: an `http://` `whisper_url` on a non-loopback host fails validation naming `whisper_url`; `http://` on localhost/127.0.0.1/::1 passes; `https://` passes anywhere.
- [x] 2.2 Failing test: an `http://` translate backend base URL on a non-loopback host fails validation naming the backend's key; loopback passes.
- [x] 2.3 Implement until 2.1 and 2.2 pass.

## 3. Verification

- [x] 3.1 Full test suite passes; a Recording against a local whisper server still starts and stops cleanly.
