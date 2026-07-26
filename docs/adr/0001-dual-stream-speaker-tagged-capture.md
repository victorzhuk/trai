---
status: accepted
---

# Capture mic and monitor as two separate streams

A meeting transcript that omits your own questions and answers is half a record, and a single mixed stream cannot say who spoke. We capture the microphone and the output sink's monitor as two independent PipeWire streams, run voice activity detection and whisper on each separately, and tag every Segment `me` or `them`, interleaving the two by start timestamp into one transcript.

## Considered options

A single mixed stream was the cheaper build — one capture, one pipeline, half the whisper load — but produces an unattributed wall of text. Monitor-only was cheapest and drops your own speech entirely.

## Consequences

Whisper load doubles. Measured headroom on this machine: 10s of audio transcribes in ~1.3-1.5s and concurrent requests serialize on the server's single context, giving ~7.8x realtime aggregate against a 2x realtime demand — so the cost is affordable, but it is the first thing to reconsider if a heavier model is swapped in.

Ordering across the two streams is not guaranteed by arrival: whisper replies come back out of order, so Segment position must come from its start timestamp, never from response order. The on-disk layout and the panel row model both assume the speaker tag exists; removing it later is not a config change.

This is speaker *tagging by capture source*, not diarization. Several remote participants sharing one meeting app all arrive as `them`.
