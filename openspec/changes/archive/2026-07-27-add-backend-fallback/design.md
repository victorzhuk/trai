## Context

The default first backend is LM Studio on a LAN machine with a discrete GPU; the fallback is a local model on the machine running the application. During grilling the LAN host could not be probed at all — the address is unknown and, if the service is bound inside WSL, it needs a port-proxy rule on the Windows side before any peer can reach it. So the fallback path is not a rare edge case here; it is the likely path on day one.

## Goals / Non-Goals

**Goals:**

- A meeting keeps producing translations when the preferred machine is unavailable.
- The user can tell, while reading, which lines came from the weaker model.
- Recovery is automatic — plugging the good machine back in does not require restarting a Recording.

**Non-Goals:**

- Retrying or backfilling Segments that failed on every backend. The PRD puts that out of scope.
- Load balancing or splitting traffic across backends. The list is a priority order, not a pool.
- Health checks that run when no Recording is active.

## Decisions

**Failover on unavailability, not on rejection.** Connection refusal, DNS failure, timeouts, and server-side errors move translation to the next backend, because they mean *this backend cannot serve right now*. A rejection naming an unknown model or a refused key is a configuration mistake: demoting silently would hide it, and the fallback would then serve every line of every meeting while the user believes the fast machine is working. Those surface as an error on the affected row and leave the backend in place.

**Degraded is a property of the Segment, not of the session.** It is recorded per translated Segment and persisted, so reopening a Recording later still shows which lines came from a fallback. A session-level banner would be lost the moment the primary recovered mid-meeting.

**Re-probe with a cheap request, not a real line.** While degraded, higher-priority backends are probed on an interval with a lightweight request rather than by attempting a real translation. Probing with real content would mean either wasting a line or translating it twice, and a probe that fails would then damage a line the fallback could have handled.

**Timeout is configuration, not a constant.** The LAN machine on a good day answers far faster than the local CPU model; a timeout tuned for one is wrong for the other. It is a config value, applied per request.

## Risks / Trade-offs

- **A flapping primary could oscillate.** If the LAN host answers a probe and then times out on real work, translation bounces between backends and lines alternate between marked and unmarked. Mitigated by the probe interval; if it proves annoying in practice the fix is a consecutive-success requirement before promoting, which this change deliberately does not build up front.
- **The fallback model's quality is untested for the target language pair.** Degraded marking is what makes that visible rather than assumed.
- **Every backend down leaves rows untranslated for the rest of the meeting.** Accepted: retry and backfill are out of scope, and the transcription panel — the part that cannot be reconstructed later — is unaffected.
