# Tasks

## 1. Confirmation dialog

- [x] 1.1 Activating force stop while Segments are transcribing, translating, or failed opens a confirmation stating the count of in-flight work to be discarded. Acceptance: the dialog names the count and the discard is irreversible.
- [x] 1.2 Dismissing the confirmation (Cancel or Escape) leaves the draining Recording untouched; confirming signals force stop exactly as the unconfirmed button did.
- [x] 1.3 With nothing in flight, force stop applies immediately without a dialog.

## 2. Verification

- [x] 2.1 Full test suite passes; app smoke-run renders.
