Thanks for the contribution.

## Summary

One or two sentences describing the change.

## Linked issue

Fixes #<!-- issue number -->. If there is no issue, say so.

## Test plan

How was this verified? Paste the exact `make` invocation(s) you ran
and the result, or describe the manual scenario if there is no
automated test (e.g. UI-only changes).

- [ ] `make fmt-check`
- [ ] `make lint` — `cargo clippy --all-targets -- -D warnings` is clean
- [ ] `make test` — the default (no-network) test suite passes

## Docs and changelog

- [ ] `README.md` updated, if user-visible behaviour changed
- [ ] `docs/prd/` or `docs/adr/` updated, if architecture or
      requirements changed
- [ ] `CONTEXT.md` glossary updated, if a new term is introduced or
      an existing one is redefined
- [ ] `CHANGELOG.md` entry under `[Unreleased]`, if user-visible

## Privacy and security

- [ ] No raw meeting audio, transcripts, or bearer keys were added
      to a log path, fixture, or test artefact
- [ ] No new network endpoint was added without a `config.toml` knob
      and a matching `config.toml.example` entry
