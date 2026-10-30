# gli: the walls.
#
# Philosophy: we do not rely on remembering best practices. We encode them as
# DETERMINISTIC checks that pass or fail, and loop until they pass. `make check`
# is the single gate; CI runs exactly it.
#
#   make check     fmt + clippy(-D warnings) + tests + coverage floor
#   make cov       human-readable coverage report
#   make cov-html  browsable HTML coverage under target/llvm-cov/html
#   make package   verify the crate packages for publish (no registry needed)
#
# Rule: never lower a threshold to make a check pass. Fix the code, or change the
# threshold ON PURPOSE in a commit that says why. COV_FLOOR is a ratchet.

# Line-coverage floor. Ratchet UP as slices land; never down to pass.
# 2026-10-30: set to 85 (measured 86.3%).
COV_FLOOR ?= 85

.PHONY: check fmt fmt-check lint test cov-gate cov cov-html build package clean

check: fmt-check lint test cov-gate
	@echo "═══ make check: ALL WALLS GREEN ═══"

fmt:
	cargo fmt

fmt-check:
	@echo "── fmt ──"
	cargo fmt --check

lint:
	@echo "── clippy (warnings are errors) ──"
	cargo clippy --all-targets -- -D warnings

test:
	@echo "── tests ──"
	cargo test --quiet

# Coverage is a wall: the build fails below COV_FLOOR, so rot cannot creep in.
cov-gate:
	@echo "── coverage floor ($(COV_FLOOR)% lines) ──"
	cargo llvm-cov --summary-only --fail-under-lines $(COV_FLOOR)

cov:
	cargo llvm-cov --summary-only

cov-html:
	cargo llvm-cov --html
	@echo "open target/llvm-cov/html/index.html"

build:
	cargo build --release

# Validate that the crate packages cleanly (manifest, README/LICENSE, file set)
# without needing the registry, so publish blockers surface before publish.
package:
	cargo publish --dry-run --locked

clean:
	cargo clean
