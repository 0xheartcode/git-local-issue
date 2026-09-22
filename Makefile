# Deterministic quality gate for gli. `make check` must be green before commit.
.PHONY: check fmt fmt-check lint test coverage build package clean

check: fmt-check lint test

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

lint:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

# Coverage is reported, not gated: it surfaces rot without flaking the build.
coverage:
	cargo llvm-cov --summary-only

build:
	cargo build --release

# Packaging gate: verify the crate builds as a publishable package (catches
# missing files, path-only deps, and metadata errors) without publishing.
package:
	cargo publish --dry-run --locked

clean:
	cargo clean
