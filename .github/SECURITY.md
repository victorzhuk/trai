# Security policy

## Scope

`trai` captures microphone and output-monitor audio on Linux,
transcribes it through a configurable backend, translates it through
configurable OpenAI-compatible chat backends, and writes the result
to a local `store_root`. Anything that touches the audio path, the
on-disk `store_root` contents, or the bearer key passed to a
configured backend is in scope.

## Supported versions

Security fixes ship on `master`. Tagged releases are
**not** maintained; `v0.1.0` and `v0.1.1` will not receive
backports. If you cannot run `master`, build from the latest tag
yourself and apply the fix on top, or wait for the next tag.

## Reporting a vulnerability

This repository has **private vulnerability reporting enabled** under
*Settings → Code security and analysis*. Confidential disclosures should
go through GitHub's private advisory flow — **not** through public
issues, DMs, or email. The verified entry point is:

- **File a private security advisory:**
  <https://github.com/victorzhuk/trai/security/advisories/new>

Draft advisories are visible only to the reporter and the repository
maintainer until the maintainer accepts and publishes (or declines)
them. This is the only confidential reporting channel for `trai`. Do
not assume any other channel — public issues, GitHub DMs, third-party
contact forms, or email — is private.

Until a private advisory is filed, do not send secrets, real audio,
transcripts, live configurations, or bearer keys to any address, DM,
or issue. Public issue content cannot be made private after the fact,
and content posted publicly may remain available through notifications,
caches, or edit history even if edited later.

> Code-of-conduct reports and other private conduct matters are
> handled separately. `dev@victorzh.uk` is the contact listed in
> `README.md` for conduct only and is **not** a security channel —
> do not send vulnerability disclosures there.

### What to file where

**Confidential vulnerabilities** — anything whose reproduction needs
real meeting audio, real transcripts, real bearer keys, or any other
sensitive data, or whose proof of concept would itself leak sensitive
data — must be filed as a **private security advisory** at the link
above. GitHub's private advisory flow is the only confidentiality
guarantee this repository offers.

**Non-sensitive bugs already observable from a public release** —
build problems, crashes that do not involve real audio, behavioural
bugs that reproduce against synthetic fixtures, or any issue whose
reproduction does not require sensitive data — may be filed as a
public GitHub issue with the `[security]` title prefix. Include:

1. A short description of the impact.
2. Reproduction steps or a minimal config with any keys redacted.
3. `git rev-parse HEAD` (the binary has no `--version` flag) and
   the relevant backend versions (whisper.cpp / LM Studio / Ollama /
   …).

Public issues are not confidential. Anything posted there may persist
in notifications, web caches, search-engine indexes, and edit history
even after the issue is closed or the content is edited.

### What to expect

- **No SLA.** This is a single-maintainer project; acknowledgement
  may take time on either channel.
- Triage happens in the advisory or issue thread.
- Fixes ship on `master`. Whether a `CHANGELOG.md` entry is added,
  and what it says, is at the maintainer's discretion.
- Credit, if given, is at the maintainer's discretion. Anonymity is
  respected on request.
- There is no paid bug-bounty programme.

### What not to do

- Do not post real audio, transcripts, or live API keys in a public
  issue. Public issue content may remain available through
  notifications, caches, or edit history even if edited later, and
  the maintainer will ask you to rotate and redact.
- Do not file a CVE request before the maintainer has agreed on the
  scope of the disclosure.

## Out of scope

- Bugs in third-party backends (whisper.cpp, Ollama, LM Studio,
  OpenAI cloud services, …). Report them upstream and link the
  issue here if it affects `trai` behaviour.
- Crashes that need an unreachable whisper or translate backend to
  reproduce, unless the crash itself is the security boundary being
  crossed.
- "Meeting audio is sensitive" in general — that is a documented
  property of the software, not a per-report vulnerability. See
  [`README.md`](../README.md) for the privacy warning around
  `whisper_kind = "openai"`.
