## Why

Force stop abandons every Segment still waiting on a transcription or translation backend. Today a single click on it discards that in-flight work instantly, with no confirmation and no count of what is being thrown away. The button now reads as dangerous, but a misclick during a long stop still costs the tail of the meeting.

## What Changes

- When in-flight work remains (Segments transcribing, translating, or failed), activating force stop opens a confirmation naming how much work will be discarded; confirming abandons it, dismissing keeps waiting.
- When nothing is in flight, force stop applies immediately with no confirmation.

## Capabilities

### Modified Capabilities

- `transcript-view`: force stop may require confirmation before discarding in-flight work

## Impact

Pure interaction change in the stop flow. Stop itself, force-stop semantics after confirmation, and all storage formats are unchanged.
