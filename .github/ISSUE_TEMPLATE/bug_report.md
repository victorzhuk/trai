---
name: Bug report
about: Something trai did or didn't do is wrong.
title: "[bug] "
labels: ["bug"]
assignees: []
---

Thanks for filing a bug. Please fill in the sections below — every
one helps reproduce the issue faster.

## Summary

One or two sentences: what went wrong?

## Reproduction steps

1.
2.
3.

A minimal `config.toml` (with any keys redacted) belongs here. If the
bug depends on a particular audio source, name the source by the
`pactl list sources` *Name* field, not the *Description*.

## Expected behaviour

What should have happened?

## Actual behaviour

What happened instead? Paste the relevant output, including any error
text shown in the Slint UI status chips.

## Environment

- **OS:** (e.g. Ubuntu 25.04, Fedora 42, Arch — kernel version if it's a PipeWire issue)
- **PipeWire version:** `pw-cli --version` or your distro's package version
- **trai version:** `git rev-parse HEAD` (the binary has no `--version` flag)
- **Rust toolchain:** `rustc --version` (only if the bug is build-related)
- **Transcription backend:** `whisper_kind` value, backend name and version
- **Translation backends:** the `base_url` and `model` of each entry in `[[translate.backends]]` (omit API keys)

## Anything else

Screenshots of the Slint UI are welcome for visual issues. Crop or redact
any meeting transcripts, participant names, or other sensitive UI content
before attaching — bug reports are public, and what is shown in the
screenshot ends up indexed alongside the issue.

## What not to paste here

Do not paste raw meeting audio, transcripts, bearer tokens, or
unredacted config snippets into the issue body. Describe the
relevant field by name or paste a path; the maintainer will ask for
a specific redacted excerpt if one is needed.
