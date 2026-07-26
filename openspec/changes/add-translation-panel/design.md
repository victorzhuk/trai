## Context

The spine appends a Segment to the transcript file the moment its transcription finalizes, which is what makes a crash survivable. A translation for that Segment arrives strictly later — a network round-trip after the original is already on disk and on screen. Whisper's own translate task was rejected during grilling because it can only produce English, and the target language is configurable.

## Goals / Non-Goals

**Goals:**

- A translated line appears beside its original without the user wondering whether the line was dropped.
- The transcript file stays append-only despite translations arriving out of band.
- Translation quality holds across a conversation, not just within one sentence.

**Non-Goals:**

- Fallback between backends and Degraded marking. The next change owns those; here a failed translation simply leaves the placeholder unresolved.
- Per-Recording target language. One target language, from configuration.
- Re-translating a Recording after it has been stopped.

## Decisions

**Translations are separate append-only records, not rewrites.** The transcript file carries two record types: a Segment record written when transcription finalizes, and a translation record written when translation returns, referring to the Segment by its identifier. A reader merges them, last write winning per Segment. Rewriting the Segment's line in place would mean seeking and rewriting a file that is being appended to concurrently, and would give up the property that a truncated tail is the only damage a crash can do.

**Placeholder rows, allocated at Segment close.** Both panels receive their row when the Segment closes, so row indices stay aligned and scrolling can be locked. The right-hand row shows a pending state until its translation returns. The alternative — appending to each panel independently as text arrives — drifts the panels apart during a fast exchange, which is exactly when reading one against the other matters.

**One request per Segment, with three lines of context.** Batching several Segments per request would give the model more context and cost fewer requests, but fills the right panel in bursts up to ten seconds behind the left, and a failed batch loses five lines instead of one. Three previous translated lines were chosen as the rolling window: enough to carry a pronoun or a name across turns, small enough that the prompt stays trivial next to the model's context.

**Context is previous translations, not previous originals.** The model sees what it already produced, which is what keeps terminology stable — the same term translated the same way twice.

**Trait seam at `Translator`.** Mirrors `Transcriber` from the spine. The pipeline core depends on the trait; tests drive it with a fake that can be made slow or made to fail, which is what the next change needs to test fallback at all.

## Risks / Trade-offs

- **A failed translation leaves a placeholder unresolved for the life of the Recording.** Accepted here deliberately: the fallback change immediately after this one is what makes failures recoverable, and inventing a partial retry now would be thrown away.
- **Rolling context propagates a bad translation.** If one line is translated poorly, the next three see it. Mitigated by the window being small.
- **Two record types make every reader merge.** The history change reads this file too, and must implement the same merge. Worth the append-only property.
