.PHONY: build check fmt test docker-build docker-run

build:
	cargo build

check:
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all-features

fmt:
	cargo fmt

test:
	cargo test

docker-build:
	docker build -t searchworks-mcp:dev .

docker-run:
	docker run --rm -p 3000:3000 searchworks-mcp:dev
