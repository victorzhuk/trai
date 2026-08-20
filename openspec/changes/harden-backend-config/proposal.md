## Why

The whisper client is built with no request timeout, and configuration has no knob for one. A hung or dead whisper server blocks a Segment submission thread forever, and because `Recording::stop()` joins every submission thread, the stop flow never returns: the user cannot end a Recording cleanly and must kill the process. The translate backends got a per-request timeout in their configuration; the transcriber did not.

URL validation at startup checks syntax only, so `http://` passes for both the whisper URL and translate backend base URLs. The translate client attaches `Authorization: Bearer <api_key>` and the whisper client POSTs raw meeting audio; over plain HTTP both cross the network in cleartext to whatever host was configured.

## What Changes

- The TOML configuration carries a whisper per-request timeout, validated at startup like the other durations, and the whisper client is built with it.
- Startup validation rejects `http://` URLs for non-loopback hosts on both the whisper URL and every translate backend base URL, naming the offending key. Loopback hosts (localhost, 127.0.0.1, ::1) keep working for local servers.

## Capabilities

### Modified Capabilities

- `app-config`: new whisper timeout key, transport-scheme validation

## Impact

Existing configurations pointing at local servers (`http://127.0.0.1:...`) are unaffected. Configurations pointing at remote hosts over plain HTTP start failing validation at startup with a message naming the key — that is the intended break. No change to on-disk formats or runtime behavior beyond requests now having an upper bound.
