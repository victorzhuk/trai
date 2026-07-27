CARGO ?= cargo
CONFIG ?= config.toml
CLIPPY_FLAGS := --all-targets -- -D warnings

.DEFAULT_GOAL := help

.PHONY: help
help: ## List available targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "} {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

.PHONY: build
build: ## Compile the debug binary
	$(CARGO) build

.PHONY: release
release: ## Compile the optimized binary
	$(CARGO) build --release

.PHONY: run
run: $(CONFIG) ## Run the app against config.toml
	$(CARGO) run -- $(CONFIG)

# Guard for `run`: the binary exits immediately without a config, so fail
# with the fix instead of a runtime error.
$(CONFIG):
	@echo "$(CONFIG) is missing; run 'make config' and fill in the placeholders" >&2
	@exit 1

.PHONY: config
config: ## Create config.toml from config.toml.example if absent
	@if [ -f $(CONFIG) ]; then \
		echo "$(CONFIG) already exists, leaving it alone"; \
	else \
		cp config.toml.example $(CONFIG); \
		echo "created $(CONFIG) — replace every placeholder before running"; \
	fi

.PHONY: fmt
fmt: ## Format the tree
	$(CARGO) fmt

.PHONY: fmt-check
fmt-check: ## Fail if the tree is unformatted
	$(CARGO) fmt --check

.PHONY: lint
lint: ## Clippy over all targets, warnings as errors
	$(CARGO) clippy $(CLIPPY_FLAGS)

.PHONY: test
test: ## Unit + integration tests, no network
	$(CARGO) test

.PHONY: test-unit
test-unit: ## Library unit tests only
	$(CARGO) test --lib

.PHONY: test-smoke
test-smoke: ## Ignored live-backend smoke tests, needs local translate backends
	$(CARGO) test --test live_backend_smoke -- --ignored --nocapture

.PHONY: check
check: fmt-check lint test ## Verification floor: fmt-check, lint, test

.PHONY: clean
clean: ## Remove build artifacts
	$(CARGO) clean
