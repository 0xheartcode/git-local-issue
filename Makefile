# Deterministic quality gate for gli. `make check` must be green before commit.
.PHONY: check fmt fmt-check lint test coverage build clean

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

clean:
	cargo clean
