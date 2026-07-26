## Why

A transcript in the original language solves half the problem. The meetings this exists for are held in a language you follow but would rather not reconstruct from memory, so the transcript is only useful next to what it means. The spine already produces ordered, attributed Segments; this change turns each of them into a translated line and puts it on screen beside the original.

## What Changes

- Every Segment is translated into the single target language configured in TOML, using an OpenAI-compatible chat endpoint.
- The translation request carries the Segment's text plus the previous three translated lines as rolling context, so pronouns, names and recurring jargon stay consistent across a conversation.
- The window gains a second panel. When a Segment closes, both panels receive a row at the same index — the original on the left, a pending placeholder on the right, replaced in place when the translation returns.
- The two panels scroll together, so a line can be read against its translation.
- Translations are persisted as their own append-only records referring to the Segment they translate, keeping the transcript file append-only while translation arrives after the original.

## Capabilities

### New Capabilities

- `translation`: turning a Segment's text into the configured target language, including the prompt's rolling context and the persistence of translated text

### Modified Capabilities

- `transcript-view`: gains the translation panel, the pending placeholder row, and locked scrolling between panels
- `app-config`: gains the target language and the translate backend definition

## Impact

Adds an outbound HTTP dependency on an OpenAI-compatible chat endpoint. Extends the transcript file with a second record type, which every reader of a Recording must now merge. No change to capture, segmentation, or transcription.
